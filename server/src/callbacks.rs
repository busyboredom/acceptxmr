use std::{str::FromStr, time::Duration};

use acceptxmr::Invoice;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{
    body::Incoming,
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        uri::InvalidUri,
        StatusCode,
    },
    Method, Request, Response, Uri,
};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::TokioExecutor,
};
use log::{debug, error, info};
use serde_json::json;
use thiserror::Error;
use tokio::{
    sync::mpsc::{channel, error::SendError, Sender},
    time::timeout,
};

use crate::server::api::{InvoiceDescription, InvoiceUpdate};

/// Delay before retrying a callback, in seconds.
const CALLBACK_RETRY_DELAY: u64 = 5;

#[derive(Debug, Clone)]
pub(crate) struct CallbackClient {
    client: Client<HttpsConnector<HttpConnector>, Full<Bytes>>,
    timeout: Duration,
}

impl CallbackClient {
    /// Returns a payment gateway client.
    pub(crate) fn new(total_timeout: Duration, connection_timeout: Duration) -> CallbackClient {
        let mut hyper_connector = HttpConnector::new();
        hyper_connector.set_connect_timeout(Some(connection_timeout));
        hyper_connector.enforce_http(false);

        let rustls_connector = HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .wrap_connector(hyper_connector);
        let client = Client::builder(TokioExecutor::new()).build(rustls_connector);

        CallbackClient {
            client,
            timeout: total_timeout,
        }
    }

    /// Call the invoices callback, if one exists. Return Ok(true) if the
    /// callback was called, or Ok(false) if there was no callback to call.
    pub(crate) async fn callback(&self, invoice: &Invoice) -> Result<bool, CallbackError> {
        let description: InvoiceDescription = serde_json::from_str(invoice.description())
            .map_err(CallbackError::InvalidDescription)?;
        let callback_uri = match description.callback {
            Some(uri) => Uri::from_str(&uri).map_err(CallbackError::InvalidCallback)?,
            None => return Ok(false),
        };

        let invoice_update: InvoiceUpdate = invoice.clone().into();
        self.request(json! {invoice_update}, &callback_uri).await?;

        Ok(true)
    }

    async fn request(
        &self,
        body: serde_json::Value,
        uri: &Uri,
    ) -> Result<Response<Incoming>, CallbackError> {
        let mut response = None;
        timeout(self.timeout, async {
                let request_builder = Request::builder()
                    .method(Method::POST)
                    .header(ACCEPT, "*/*")
                    .header(CONTENT_TYPE, "application/json")
                    .uri(uri);

                let request = match request_builder.body(Full::new(body.to_string().into())) {
                    Ok(r) => r,
                    Err(e) => {
                        response = Some(Err(e.into()));
                        return;
                    }
                };

                match self.client.request(request).await {
                    Ok(r) if r.status().is_server_error() | r.status().is_client_error() => {
                        error!(
                            "Callback response contains an error. Callback will be retried. Status code: {}, body: {:?}",
                            r.status(),
                            r.body()
                        );
                        response = Some(Err(r.into()));
                    }
                    Ok(r) => {
                        debug!("Callback successful. Response: {r:?}");
                        response = Some(Ok(r));
                    }
                    Err(e) => {
                        error!("Error calling callback, retrying: {}", e);
                        response = Some(Err(CallbackError::Request(Box::new(e))));
                    }
                };
        })
        .await
        .map_err(|_| CallbackError::Timeout)?;
        response.unwrap_or(Err(CallbackError::Timeout))
    }
}

impl Default for CallbackClient {
    fn default() -> Self {
        CallbackClient::new(Duration::from_secs(10), Duration::from_secs(5))
    }
}

pub(crate) struct CallbackQueue {
    sender: Sender<CallbackCommand>,
}

impl CallbackQueue {
    pub(crate) fn init(client: CallbackClient, queue_size: usize) -> CallbackQueue {
        let (sender, mut receiver) = channel(queue_size);
        let queue = CallbackQueue {
            sender: sender.clone(),
        };

        tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Some(CallbackCommand::Shutdown) => {
                        info!("Callback queue received shutdown signal");
                        break;
                    }
                    Some(CallbackCommand::Call { invoice, delay }) => {
                        debug!("Processing callback for invoice with ID {}", invoice.id());
                        let client_clone = client.clone();
                        let sender_clone = sender.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(delay).await;
                            if let Err(e) = client_clone.callback(&invoice).await {
                                error!("Failed to call callback: {}. Callback will be retried.", e);
                                sender_clone
                                    .send(CallbackCommand::Call {
                                        invoice,
                                        delay: Duration::from_secs(CALLBACK_RETRY_DELAY),
                                    })
                                    .await
                                    .expect("failed to place callback back in the callback queue");
                            }
                        });
                    }
                    None => {
                        info!("Callback queue sender closed. Stopping callback queue.");
                        break;
                    }
                }
            }
        });

        info!("Callback queue initialized");
        queue
    }

    pub(crate) async fn send(
        &self,
        command: CallbackCommand,
    ) -> Result<(), SendError<CallbackCommand>> {
        self.sender.send(command).await
    }
}

pub(crate) enum CallbackCommand {
    // TODO: Implement graceful shutdown.
    #[allow(unused)]
    Shutdown,
    Call {
        invoice: Invoice,
        delay: Duration,
    },
}

#[derive(Error, Debug)]
pub(crate) enum CallbackError {
    #[error("HTTP request failed: {0}")]
    Request(Box<dyn std::error::Error + Send + Sync>),
    #[error("failed to build HTTP Request: {0}")]
    InvalidRequest(#[from] hyper::http::Error),
    #[error("Callback recipient returned an error. Status code: {status}, body: {body:?}")]
    Response { status: StatusCode, body: Incoming },
    #[error("HTTP request timed out")]
    Timeout,
    #[error("failed to deserialize invoice description: {0}")]
    InvalidDescription(serde_json::Error),
    #[error("callback is not a valid URI: {0}")]
    InvalidCallback(InvalidUri),
}

impl From<Response<Incoming>> for CallbackError {
    fn from(value: Response<Incoming>) -> Self {
        CallbackError::Response {
            status: value.status(),
            body: value.into_body(),
        }
    }
}
