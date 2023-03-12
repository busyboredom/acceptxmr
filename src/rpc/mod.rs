mod authentication;

use std::{
    any,
    collections::HashSet,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use authentication::{AuthError, AuthInfo};
use http::StatusCode;
use hyper::{
    body,
    client::connect::HttpConnector,
    header::{AUTHORIZATION, WWW_AUTHENTICATE},
    Body, Method, Request, Uri,
};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use log::{debug, trace, warn};
use monero::consensus::{deserialize, encode};
use serde_json::json;
use thiserror::Error;
use tokio::time::{error, timeout};

/// Maximum number of transactions to request at once (daemon limits this).
const MAX_REQUESTED_TRANSACTIONS: usize = 100;

#[derive(Debug, Clone)]
pub(crate) struct RpcClient {
    client: hyper::Client<HttpsConnector<HttpConnector>>,
    url: Uri,
    timeout: Duration,
    auth_info: Arc<Mutex<Option<AuthInfo>>>,
}

impl RpcClient {
    /// Returns an Rpc client pointing at the specified monero daemon.
    pub fn new(
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
        let rustls_connector = HttpsConnectorBuilder::new()
            .with_native_roots()
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .wrap_connector(hyper_connector);
        let client = hyper::Client::builder().build(rustls_connector);
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

    pub async fn block(&self, height: u64) -> Result<(monero::Hash, monero::Block), RpcError> {
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

    pub async fn block_transactions(
        &self,
        block: &monero::Block,
    ) -> Result<Vec<monero::Transaction>, RpcError> {
        // Get block transactions in sets of 100 or less (the restricted RPC maximum).
        let transaction_hashes = &block.tx_hashes;
        self.transactions_by_hashes(transaction_hashes).await
    }

    pub async fn txpool(&self) -> Result<Vec<monero::Transaction>, RpcError> {
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

    pub async fn txpool_hashes(&self) -> Result<HashSet<monero::Hash>, RpcError> {
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

    pub async fn transactions_by_hashes(
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

    pub async fn daemon_height(&self) -> Result<u64, RpcError> {
        let request_body = r#"{"jsonrpc":"2.0","id":"0","method":"get_block_count"}"#;
        let request_endpoint = "json_rpc";

        let res = self.request(request_body, request_endpoint).await?;

        let count = res["result"]["count"]
            .as_u64()
            .ok_or_else(|| RpcError::MissingData("{{ result: {{ count: \"...\" }}".to_string()))?;

        Ok(count)
    }

    async fn request(&self, body: &str, endpoint: &str) -> Result<serde_json::Value, RpcError> {
        let mut req = Request::builder()
            .method(Method::POST)
            .uri(self.url.clone().to_string() + endpoint)
            .body(Body::from(body.to_owned()))?;
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
        let mut response = timeout(self.timeout, self.client.request(req)).await??;

        // If response has www-authenticate header and 401 status, perform digest
        // authentication.
        if response.status() == StatusCode::UNAUTHORIZED
            && response.headers().contains_key(WWW_AUTHENTICATE)
        {
            debug!("Recieved 401 UNAUTHORIZED response. Performing digest authentication.");
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
                .body(Body::from(body.to_owned()))?;
            // Await full response.
            response = timeout(self.timeout, self.client.request(req)).await??;
        }

        let (_parts, body) = response.into_parts();
        let full_body = body::to_bytes(body).await?;

        Ok(serde_json::from_slice(&full_body)?)
    }

    pub fn url(&self) -> String {
        self.url.clone().to_string()
    }
}

#[allow(clippy::module_name_repetitions)]
#[derive(Error, Debug)]
pub enum RpcError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] hyper::Error),
    #[error("failed to build HTTP request: {0}")]
    Request(#[from] hyper::http::Error),
    #[error("HTTP request timed out: {0}")]
    Timeout(#[from] error::Elapsed),
    #[error("hex decoding failed: {0}")]
    HexDecode(#[from] hex::FromHexError),
    #[error("(de)serialization failed: {0}")]
    Serialization(#[from] encode::Error),
    #[error("expected data was not present in RPC response, or was the wrong data type: {0}")]
    MissingData(String),
    #[error("failed to interpret json value \"{found}\" from RPC response as {expected}")]
    DataType {
        found: serde_json::Value,
        expected: &'static str,
    },
    #[error("failed to interpret response body as json: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("authentication error: {0}")]
    Auth(#[from] AuthError),
}
