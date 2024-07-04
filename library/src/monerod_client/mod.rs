mod authentication;

use std::{
    any,
    collections::HashSet,
    fs::File,
    future::Future,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use authentication::{AuthError, AuthInfo};
use backoff::{backoff::Backoff, ExponentialBackoffBuilder};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{
    header::{AUTHORIZATION, WWW_AUTHENTICATE},
    http::StatusCode,
    Method, Request, Uri,
};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client as HttpClient},
    rt::TokioExecutor,
};
use log::{debug, trace, warn};
use monero::consensus::{deserialize, encode};
use serde_json::json;
use thiserror::Error;
use tokio::time::{error, timeout};

/// Maximum number of transactions to request at once (daemon limits this).
const MAX_REQUESTED_TRANSACTIONS: usize = 100;

/// A monerod RPC client.
#[derive(Debug, Clone)]
pub struct RpcClient {
    client: HttpClient<HttpsConnector<HttpConnector>, Full<Bytes>>,
    url: Uri,
    timeout: Duration,
    auth_info: Arc<Mutex<Option<AuthInfo>>>,
}

impl RpcClient {
    /// Returns an Rpc client pointing at the specified monero daemon.
    pub(crate) fn new(
        url: Uri,
        total_timeout: Duration,
        connection_timeout: Duration,
        username: Option<String>,
        password: Option<String>,
        seed: Option<u64>,
    ) -> RpcClient {
        let mut hyper_connector = HttpConnector::new();
        hyper_connector.set_connect_timeout(Some(connection_timeout));
        hyper_connector.enforce_http(false);
        hyper_connector.set_keepalive(Some(Duration::from_secs(25)));
        let rustls_connector = HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .wrap_connector(hyper_connector);
        let client = HttpClient::builder(TokioExecutor::new()).build(rustls_connector);
        let auth_info = Arc::new(Mutex::new(if username.is_some() || password.is_some() {
            Some(AuthInfo::new(
                username.unwrap_or_default(),
                password.unwrap_or_default(),
                seed,
            ))
        } else {
            None
        }));

        RpcClient {
            client,
            url,
            timeout: total_timeout,
            auth_info,
        }
    }

    async fn request(&self, body: &str, endpoint: &str) -> Result<serde_json::Value, RpcError> {
        let mut req = Request::builder()
            .method(Method::POST)
            .uri(self.url.clone().to_string() + endpoint)
            .body(Full::new(body.to_owned().into()))?;
        let (method, uri) = (req.method().clone(), req.uri().clone());

        // If configured with a username and password, try to authenticate with most
        // recent nonce.
        if let Some(auth_info) = &mut *self
            .auth_info
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        {
            if let Some(auth_header) = auth_info.authenticate(&uri, &method)? {
                req.headers_mut().insert(AUTHORIZATION, auth_header);
            }
        }

        // Await full response.
        let mut response = timeout(self.timeout, self.client.request(req))
            .await?
            .map_err(|e| RpcError::Request(Box::new(e)))?;

        // If response has www-authenticate header and 401 status, perform digest
        // authentication.
        let mut exponential_backoff = ExponentialBackoffBuilder::default()
            .with_max_elapsed_time(None)
            .with_max_interval(Duration::from_secs(30))
            .build();
        while response.status() == StatusCode::UNAUTHORIZED
            && response.headers().contains_key(WWW_AUTHENTICATE)
        {
            debug!("Received 401 UNAUTHORIZED response. Performing digest authentication.");
            let auth_header = self
                .auth_info
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_mut()
                .ok_or(AuthError::Unauthorized)?
                .authenticate_with_resp(&response, &uri, &method)?;
            let req = Request::builder()
                .method(Method::POST)
                .uri(self.url.clone().to_string() + endpoint)
                .header(AUTHORIZATION, auth_header)
                .body(Full::new(body.to_owned().into()))?;
            // Await full response.
            response = timeout(self.timeout, self.client.request(req))
                .await?
                .map_err(|e| RpcError::Request(Box::new(e)))?;

            #[allow(clippy::expect_used)]
            tokio::time::sleep(
                exponential_backoff
                    .next_backoff()
                    .expect("RPC exponential backoff timed out. This is a bug."),
            )
            .await;
        }

        let (_parts, body) = response.into_parts();

        Ok(serde_json::from_slice(
            &body
                .collect()
                .await
                .map_err(|e| RpcError::Request(Box::new(e)))?
                .to_bytes(),
        )?)
    }
}

impl Client for RpcClient {
    async fn block(&self, height: u64) -> Result<(monero::Hash, monero::Block), RpcError> {
        trace!("Requesting block {}", height);
        let request_body = r#"{"jsonrpc":"2.0","id":"0","method":"get_block","params":{"height":"#
            .to_owned()
            + &height.to_string()
            + "}}";
        let request_endpoint = "json_rpc";

        let res: serde_json::Value = self.request(&request_body, request_endpoint).await?;

        let block_hash_str = res["result"]["block_header"]["hash"]
            .as_str()
            .ok_or_else(|| {
                RpcError::MissingData(
                    "{{ result: {{ block_header: {{ hash: \"...\"}} }} }}".to_string(),
                )
            })?;
        let block_hash_hex = hex::decode(block_hash_str)?;
        let block_hash = monero::Hash::from_slice(&block_hash_hex);

        let block_str = res["result"]["blob"].as_str().ok_or_else(|| {
            RpcError::MissingData("{{ result: {{ blob: \"...\" }} }}".to_string())
        })?;
        let block_hex = hex::decode(block_str)?;
        let block: monero::Block = deserialize(&block_hex)?;

        Ok((block_hash, block))
    }

    async fn block_transactions(
        &self,
        block: &monero::Block,
    ) -> Result<Vec<monero::Transaction>, RpcError> {
        // Get block transactions in sets of 100 or less (the restricted RPC maximum).
        let transaction_hashes = &block.tx_hashes;
        self.transactions_by_hashes(transaction_hashes).await
    }

    async fn txpool(&self) -> Result<Vec<monero::Transaction>, RpcError> {
        trace!("Requesting txpool");
        let mut transactions = Vec::new();
        let request_body = "";
        let request_endpoint = "get_transaction_pool";

        let res = self.request(request_body, request_endpoint).await?;

        let Some(blobs) = res["transactions"].as_array() else {
            // If there are no transactions in the txpool, just return an empty list.
            return Ok(transactions);
        };
        for blob in blobs {
            let tx_str = blob["tx_blob"].as_str().ok_or_else(|| {
                RpcError::MissingData("{{ transactions: [ {{ tx_blob: \"...\" }} ] }}".to_string())
            })?;
            let tx_hex = hex::decode(tx_str)?;
            let tx: monero::Transaction = deserialize(&tx_hex)?;
            transactions.push(tx);
        }
        Ok(transactions)
    }

    async fn txpool_hashes(&self) -> Result<HashSet<monero::Hash>, RpcError> {
        trace!("Requesting txpool hashes");
        let mut transactions = HashSet::new();
        let request_body = "";
        let request_endpoint = "get_transaction_pool_hashes";

        let res = self.request(request_body, request_endpoint).await?;

        let Some(blobs) = res["tx_hashes"].as_array() else {
            // If there are no tx hashes, just return an empty list.
            return Ok(transactions);
        };
        for blob in blobs {
            let tx_hash_str = blob.as_str().ok_or_else(|| RpcError::DataType {
                found: blob.clone(),
                expected: any::type_name::<&str>(),
            })?;
            let tx_hash_hex = hex::decode(tx_hash_str)?;
            let tx_hash = deserialize(&tx_hash_hex)?;
            transactions.insert(tx_hash);
        }
        Ok(transactions)
    }

    async fn transactions_by_hashes(
        &self,
        hashes: &[monero::Hash],
    ) -> Result<Vec<monero::Transaction>, RpcError> {
        let mut transactions = Vec::new();
        for i in 0..=hashes.len() / MAX_REQUESTED_TRANSACTIONS {
            // We've gotta grab these in parts to avoid putting too much load on the RPC
            // server, so these are the start and end indexes of the hashes
            // we're grabbing for now.
            //
            // TODO: Get them concurrently.
            let starting_index: usize = i * MAX_REQUESTED_TRANSACTIONS;
            let ending_index: usize =
                std::cmp::min(MAX_REQUESTED_TRANSACTIONS * (i + 1), hashes.len());

            // If requesting an empty list, return what we have now.
            if ending_index == starting_index {
                return Ok(transactions);
            }

            // Build a json containing the hashes of the transactions we want.
            trace!("Requesting {} transactions", ending_index - starting_index);
            let request_body = r#"{"txs_hashes":"#.to_owned()
                + &json!(hashes[starting_index..ending_index]
                    .iter()
                    .map(|x| hex::encode(x.as_bytes())) // Convert from monero::Hash to hex.
                    .collect::<Vec<String>>())
                .to_string()
                + "}";
            let request_endpoint = "get_transactions";

            let res = self.request(&request_body, request_endpoint).await?;

            let hexes = res["txs_as_hex"]
                .as_array()
                .ok_or_else(|| RpcError::MissingData("{{ txs_as_hex: [...] }}".to_string()))?;
            if ending_index - starting_index == hexes.len() {
                trace!("Received {} transactions", hexes.len());
            } else {
                warn!(
                    "Received {} transactions, requested {}",
                    hexes.len(),
                    ending_index - starting_index
                );
            }

            // Add these transactions to the total list.
            for tx_json in hexes {
                let tx_str = tx_json.as_str().ok_or(RpcError::DataType {
                    found: tx_json.clone(),
                    expected: "&str",
                })?;
                let tx_hex = hex::decode(tx_str)?;
                let tx: monero::Transaction = deserialize(&tx_hex)?;
                transactions.push(tx);
            }
        }
        Ok(transactions)
    }

    async fn daemon_height(&self) -> Result<u64, RpcError> {
        let request_body = r#"{"jsonrpc":"2.0","id":"0","method":"get_block_count"}"#;
        let request_endpoint = "json_rpc";

        let res = self.request(request_body, request_endpoint).await?;

        let count = res["result"]["count"]
            .as_u64()
            .ok_or_else(|| RpcError::MissingData("{{ result: {{ count: \"...\" }}".to_string()))?;

        Ok(count)
    }

    fn url(&self) -> String {
        self.url.clone().to_string()
    }
}

/// A mocker monerod client. Returns canned responses for testing purposes.
#[derive(Debug, Copy, Clone)]
pub struct MockClient;

impl MockClient {
    pub(crate) fn new() -> MockClient {
        MockClient {}
    }
}

impl Client for MockClient {
    async fn block(&self, height: u64) -> Result<(monero::Hash, monero::Block), RpcError> {
        let block_file = File::open(format!(
            "../testing-utils/rpc_resources/blocks/{height}/block.json"
        ))
        .map_err(|e| RpcError::MissingData(e.to_string()))?;
        let res: serde_json::Value = serde_json::from_reader(block_file)?;

        let block_hash_str = res["result"]["block_header"]["hash"]
            .as_str()
            .ok_or_else(|| {
                RpcError::MissingData(
                    "{{ result: {{ block_header: {{ hash: \"...\"}} }} }}".to_string(),
                )
            })?;
        let block_hash_hex = hex::decode(block_hash_str)?;
        let block_hash = monero::Hash::from_slice(&block_hash_hex);

        let block_str = res["result"]["blob"].as_str().ok_or_else(|| {
            RpcError::MissingData("{{ result: {{ blob: \"...\" }} }}".to_string())
        })?;
        let block_hex = hex::decode(block_str)?;
        let block: monero::Block = deserialize(&block_hex)?;

        Ok((block_hash, block))
    }

    async fn block_transactions(
        &self,
        _block: &monero::Block,
    ) -> Result<Vec<monero::Transaction>, RpcError> {
        Ok(Vec::new())
    }

    async fn txpool(&self) -> Result<Vec<monero::Transaction>, RpcError> {
        Ok(Vec::new())
    }

    async fn txpool_hashes(&self) -> Result<HashSet<monero::Hash>, RpcError> {
        Ok(HashSet::new())
    }

    async fn transactions_by_hashes(
        &self,
        _hashes: &[monero::Hash],
    ) -> Result<Vec<monero::Transaction>, RpcError> {
        Ok(Vec::new())
    }

    async fn daemon_height(&self) -> Result<u64, RpcError> {
        Ok(2_477_657)
    }

    fn url(&self) -> String {
        "http://node.example.com".to_string()
    }
}

/// Necessary methods for a monerod client.
pub trait Client: Clone + Send + Sync {
    /// Fetch a block hiven its height.
    fn block(
        &self,
        height: u64,
    ) -> impl Future<Output = Result<(monero::Hash, monero::Block), RpcError>> + Send;
    /// Fetch a block's transactions.
    fn block_transactions(
        &self,
        block: &monero::Block,
    ) -> impl Future<Output = Result<Vec<monero::Transaction>, RpcError>> + Send;
    /// Fetch the txpool.
    fn txpool(&self) -> impl Future<Output = Result<Vec<monero::Transaction>, RpcError>> + Send;
    /// Fetch the hashed of all transactions in the txpool.
    fn txpool_hashes(&self)
        -> impl Future<Output = Result<HashSet<monero::Hash>, RpcError>> + Send;
    /// Fetch transactions given the hashes of those transactions.
    fn transactions_by_hashes(
        &self,
        hashes: &[monero::Hash],
    ) -> impl Future<Output = Result<Vec<monero::Transaction>, RpcError>> + Send;
    /// Fetch the blockchain height from monerod.
    fn daemon_height(&self) -> impl Future<Output = Result<u64, RpcError>> + Send;
    /// The URL of the monero daemon.
    fn url(&self) -> String;
}

/// An error originating from the monerod client.
#[derive(Error, Debug)]
pub enum RpcError {
    /// HTTP request failed.
    #[error("HTTP request failed: {0}")]
    Request(Box<dyn std::error::Error + Send + Sync>),
    /// Failed to build the HTTP request.
    #[error("failed to build HTTP Request: {0}")]
    InvalidRequest(#[from] hyper::http::Error),
    /// HTTP request timed out.
    #[error("HTTP request timed out: {0}")]
    Timeout(#[from] error::Elapsed),
    /// Failed to decode a hex value.
    #[error("hex decoding failed: {0}")]
    HexDecode(#[from] hex::FromHexError),
    /// Failed to (de)serialize.
    #[error("(de)serialization failed: {0}")]
    Serialization(#[from] encode::Error),
    /// RPC response is missing expected data.
    #[error("expected data was not present in RPC response, or was the wrong data type: {0}")]
    MissingData(String),
    /// A field in the RPC response has the wrong type.
    #[error("failed to interpret json value \"{found}\" from RPC response as {expected}")]
    DataType {
        /// The type received.
        found: serde_json::Value,
        /// The type expected.
        expected: &'static str,
    },
    /// The response is not valid json.
    #[error("failed to interpret response body as json: {0}")]
    InvalidJson(#[from] serde_json::Error),
    /// Failed to authenticate.
    #[error("authentication error: {0}")]
    Auth(#[from] AuthError),
}
