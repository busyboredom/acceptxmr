use std::{sync::Arc, time::Duration};

use http::header::{ACCEPT, CONTENT_TYPE};
use hyper::{
    client::connect::HttpConnector, header::AUTHORIZATION, Body, Method, Request, Response, Uri,
};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use log::{debug, error, LevelFilter};
use rustls::{
    client::{ServerCertVerified, ServerCertVerifier},
    ClientConfig,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::time::{error, timeout};

pub const PRIVATE_VIEW_KEY: &str =
    "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
pub const PRIMARY_ADDRESS: &str =
    "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";

#[derive(Debug, Clone)]
pub struct GatewayClient {
    client: hyper::Client<HttpsConnector<HttpConnector>>,
    pub url: Uri,
    timeout: Duration,
    pub token: Option<String>,
}

impl GatewayClient {
    /// Returns a payment gateway client.
    pub fn new(
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
                    .with_safe_defaults()
                    .with_custom_certificate_verifier(Arc::new(NoCertVerifier {}))
                    .with_no_client_auth(),
            )
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .wrap_connector(hyper_connector);
        let client = hyper::Client::builder().build(rustls_connector);

        GatewayClient {
            client,
            url,
            timeout: total_timeout,
            token,
        }
    }

    pub async fn request(&self, body: &str, endpoint: &str) -> Result<Response<Body>, ClientError> {
        let mut response = None;
        timeout(self.timeout, async {
            loop {
                let mut request_builder = Request::builder()
                    .method(Method::POST)
                    .header(ACCEPT, "*/*")
                    .header(CONTENT_TYPE, "application/json")
                    .uri(self.url.clone().to_string() + endpoint);

                if let Some(token) = &self.token {
                    request_builder =
                        request_builder.header(AUTHORIZATION, format!("Bearer {}", token));
                }

                let request = match request_builder.body(Body::from(body.to_string())) {
                    Ok(r) => r,
                    Err(e) => {
                        response = Some(Err(e.into()));
                        break;
                    }
                };

                debug!("Sending request: {:?}", request);

                match self.client.request(request).await {
                    Ok(r) => {
                        response = Some(Ok(r));
                        break;
                    }
                    Err(e) if e.is_connect() => {
                        error!("Error connecting to gateway, retrying: {}", e);
                        continue;
                    }
                    Err(e) => {
                        error!("Checkout response contains an error: {}", e);
                        response = Some(Err(e.into()))
                    }
                };
            }
        })
        .await?;
        response.expect("Timed out waiting for response.")
    }

    pub async fn checkout(&self) -> Result<Response<Body>, ClientError> {
        let endpoint = "checkout";

        #[derive(Deserialize, Serialize)]
        struct CheckoutInfo {
            message: String,
        }

        let body = r#"{"message":"This is a test message"}"#;

        self.request(body, endpoint).await
    }
}

impl Default for GatewayClient {
    fn default() -> Self {
        GatewayClient::new(
            Uri::from_static("https://localhost:8081"),
            Duration::from_secs(1),
            Duration::from_millis(500),
            Some("supersecrettoken".to_string()),
        )
    }
}

/// Initialize the logging implementation. Defaults to `Trace` verbosity for
/// `AcceptXMR` and `Warn` for dependencies.
pub fn init_logger() {
    let _ = env_logger::builder()
        .is_test(true)
        .filter_level(LevelFilter::Debug)
        .filter_module("acceptxmr", LevelFilter::Trace)
        .filter_module("acceptxmr_server", LevelFilter::Trace)
        .try_init();
}

struct NoCertVerifier {}

impl ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn request_scts(&self) -> bool {
        false
    }
}

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] hyper::Error),
    #[error("failed to build HTTP request: {0}")]
    Request(#[from] hyper::http::Error),
    #[error("HTTP request timed out: {0}")]
    Timeout(#[from] error::Elapsed),
    #[error("failed to interpret response body as json: {0}")]
    InvalidJson(#[from] serde_json::Error),
}
