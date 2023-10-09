use std::{
    net::SocketAddr,
    str::FromStr,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use acceptxmr_server::api::{types::invoice_id::Base64InvoiceId, InvoiceUpdate};
use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Full};
use hyper::{
    body::Incoming,
    header::AUTHORIZATION,
    http::header::{ACCEPT, CONTENT_TYPE},
    service::service_fn,
    Error as HyperError, Method, Request, Response, StatusCode, Uri,
};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder as ServerBuilder,
};
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc::{self, Receiver, Sender},
    time::{
        error::{self, Elapsed},
        timeout,
    },
};
use tokio_rustls::rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, Error as RustlsError, SignatureScheme,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

#[derive(Debug, Clone)]
pub(crate) struct GatewayClient {
    client: Client<HttpsConnector<HttpConnector>, Full<Bytes>>,
    pub(crate) url: Uri,
    pub(crate) timeout: Duration,
    pub(crate) token: Option<String>,
}

impl GatewayClient {
    /// Returns a payment gateway client.
    pub(crate) fn new(
        url: Uri,
        total_timeout: Duration,
        connection_timeout: Duration,
        token: Option<String>,
    ) -> GatewayClient {
        let mut hyper_connector = HttpConnector::new();
        hyper_connector.set_connect_timeout(Some(connection_timeout));
        hyper_connector.enforce_http(false);

        let rustls_connector = HttpsConnectorBuilder::new()
            .with_tls_config(
                ClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(NoCertVerifier {}))
                    .with_no_client_auth(),
            )
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .wrap_connector(hyper_connector);
        let client = Client::builder(TokioExecutor::new()).build(rustls_connector);

        GatewayClient {
            client,
            url,
            timeout: total_timeout,
            token,
        }
    }

    pub(crate) async fn request(
        &self,
        body: &str,
        endpoint: &str,
    ) -> Result<Response<Incoming>, ClientError> {
        let mut response = None;
        timeout(self.timeout, async {
            let mut request_builder = Request::builder()
                .method(Method::POST)
                .header(ACCEPT, "*/*")
                .header(CONTENT_TYPE, "application/json")
                .uri(self.url.clone().to_string() + endpoint);

            if let Some(token) = &self.token {
                request_builder = request_builder.header(AUTHORIZATION, format!("Bearer {token}"));
            }

            let request =
                match request_builder.body(Full::new(Bytes::copy_from_slice(body.as_bytes()))) {
                    Ok(r) => r,
                    Err(e) => {
                        response = Some(Err(e.into()));
                        return;
                    }
                };

            debug!("Sending request: {:?}", request);

            match self.client.request(request).await {
                Ok(r) if r.status().is_server_error() | r.status().is_client_error() => {
                    error!(
                        "Response contains an error. Status code: {}, body: {:?}",
                        r.status(),
                        r.body()
                    );
                    response = Some(Ok(r));
                }
                Ok(r) => {
                    debug!("Request successful. Response: {r:?}");
                    response = Some(Ok(r));
                }
                Err(e) => {
                    error!("Response contains an error: {}", e);
                    response = Some(Err(ClientError::Request(Box::new(e))));
                }
            };
        })
        .await?;
        response.expect("Timed out waiting for response.")
    }

    pub(crate) async fn new_invoice(
        &self,
        payload: MockNewInvoicePayload,
    ) -> Result<Response<Incoming>, ClientError> {
        let endpoint = "invoice";

        let body = serde_json::to_value(payload)
            .expect("failed to build json from new_invoice payload")
            .to_string();

        self.request(&body, endpoint).await
    }

    pub(crate) async fn subscribe_to_websocket(
        self,
        invoice_id: Base64InvoiceId,
    ) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, ClientError> {
        let websocket_endpoint = format!(
            "ws://{}:{}/invoice/ws?id={invoice_id}",
            self.url.host().expect("no host configured"),
            self.url.port().expect("no port configured")
        );
        debug!("Websocket endpoint: {}", websocket_endpoint);
        let (socket, response) = tokio_tungstenite::connect_async(websocket_endpoint)
            .await
            .map_err(|e| ClientError::WebsocketUpgradeFailure(Box::new(e)))?;

        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            return Err(ClientError::WebsocketUpgradeFailure(Box::new(format!(
                "{:?}",
                response.into_body()
            ))));
        }

        Ok(socket)
    }
}

impl Default for GatewayClient {
    fn default() -> Self {
        GatewayClient::new(
            Uri::from_static("https://localhost:8081"),
            Duration::from_secs(60),
            Duration::from_secs(30),
            Some("supersecrettoken".to_string()),
        )
    }
}

#[derive(Debug)]
struct NoCertVerifier {}

impl ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA1,
            SignatureScheme::ECDSA_SHA1_Legacy,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct MockNewInvoicePayload {
    pub(crate) piconeros_due: u64,
    pub(crate) confirmations_required: u64,
    pub(crate) expiration_in: u64,
    pub(crate) order: String,
    pub(crate) callback: Option<String>,
}

impl Default for MockNewInvoicePayload {
    fn default() -> Self {
        MockNewInvoicePayload {
            piconeros_due: 2_234_345,
            confirmations_required: 2,
            expiration_in: 20,
            order: "I am a test order".to_string(),
            callback: Some("http://localhost:1234".to_string()),
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct MockInvoiceIdPayload {
    pub(crate) invoice_id: Base64InvoiceId,
}

pub(crate) struct CallbackListener {
    address: SocketAddr,
    rx: Receiver<InvoiceUpdate>,
    tx: Sender<ListenerCommand>,
}

async fn handle(
    req: Request<Incoming>,
    tx: Sender<InvoiceUpdate>,
    rx: Arc<Mutex<Receiver<ListenerCommand>>>,
) -> Result<Response<Empty<Bytes>>, HyperError> {
    let invoice: InvoiceUpdate =
        serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes()).unwrap();
    tx.send(invoice).await.unwrap();

    // Check for any commands before responding.
    if let Ok(command) = rx.lock().unwrap_or_else(PoisonError::into_inner).try_recv() {
        match command {
            // Send a bad gateway response as if the service were down.
            ListenerCommand::FailNext => {
                let mut response = Response::new(Empty::new());
                let status = response.status_mut();
                *status = StatusCode::BAD_GATEWAY;
                return Ok::<_, HyperError>(response);
            }
        }
    }

    Ok::<_, HyperError>(Response::new(Empty::new()))
}

impl CallbackListener {
    pub(crate) async fn init() -> Self {
        let addr: SocketAddr = ([127, 0, 0, 1], 0).into();
        let listener = TcpListener::bind(addr).await.unwrap();
        let address = listener.local_addr().unwrap();

        let (update_tx, update_rx) = mpsc::channel(100);
        let (command_tx, command_rx) = mpsc::channel(100);
        let sharable_command_rx = Arc::new(Mutex::new(command_rx));

        info!("Callback listener bound to {}", address);
        tokio::spawn(async move {
            let service = service_fn(|req: Request<Incoming>| {
                handle(req, update_tx.clone(), sharable_command_rx.clone())
            });

            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let stream = TokioIo::new(stream);

                let _ = ServerBuilder::new(TokioExecutor::new())
                    .serve_connection_with_upgrades(stream, service)
                    .await;
            }
        });

        CallbackListener {
            address,
            rx: update_rx,
            tx: command_tx,
        }
    }

    pub(crate) async fn recv_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<InvoiceUpdate>, Elapsed> {
        tokio::time::timeout(timeout, self.rx.recv()).await
    }

    pub(crate) fn url(&self) -> Uri {
        Uri::from_str(&format!(
            "http://{}:{}",
            self.address.ip(),
            self.address.port()
        ))
        .unwrap()
    }

    pub(crate) fn port(&self) -> u16 {
        self.address.port()
    }

    /// Artificially fail callback to test retry mechanism.
    pub(crate) async fn fail_one_callback(&mut self) {
        self.tx.send(ListenerCommand::FailNext).await.unwrap();
    }
}

enum ListenerCommand {
    /// Fail the next callback.
    FailNext,
}

#[derive(Error, Debug)]
pub(crate) enum ClientError {
    #[error("HTTP request failed: {0}")]
    Request(Box<dyn std::error::Error + Send + Sync>),
    #[error("failed to build HTTP request: {0}")]
    InvalidRequest(#[from] hyper::http::Error),
    #[error("HTTP request timed out: {0}")]
    Timeout(#[from] error::Elapsed),
    #[error("failed to interpret response body as json: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("failed to upgrade to websocket connection: {0:?}")]
    WebsocketUpgradeFailure(Box<dyn std::fmt::Debug + Send + Sync>),
}
