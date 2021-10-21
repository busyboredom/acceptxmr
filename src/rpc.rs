use std::time::Duration;
use std::{any, fmt};
use std::{collections::HashSet, error::Error};

use log::{trace, warn};
use monero::consensus::{deserialize, encode};

/// Maximum number of transactions to request at once (daemon limits this).
const MAX_REQUESTED_TRANSACTIONS: usize = 100;

#[derive(Debug, Clone)]
pub(crate) struct RpcClient {
    client: reqwest::Client,
    url: String,
}

impl RpcClient {
    /// Returns an Rpc client pointing at the specified monero daemon.
    ///
    /// # Errors
    ///
    /// This method fails if a TLS backend cannot be initialized, or the resolver
    /// cannot load the system configuration.
    pub fn new(url: &str, connection_timeout: Duration) -> Result<RpcClient, RpcError> {
        let client = reqwest::ClientBuilder::new()
            .connect_timeout(connection_timeout)
            .build()?;
        Ok(RpcClient {
            client,
            url: url.to_string(),
        })
    }

    pub async fn block(&self, height: u64) -> Result<(monero::Hash, monero::Block), RpcError> {
        trace!("Requesting block {}", height);
        let request_body = r#"{"jsonrpc":"2.0","id":"0","method":"get_block","params":{"height":"#
            .to_owned()
            + &height.to_string()
            + "}}";
        let request_endpoint = "/json_rpc";

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
        let block = deserialize(&block_hex)?;

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
        let request_endpoint = "/get_transaction_pool";

        let res = self.request(request_body, request_endpoint).await?;

        let blobs = res["transactions"]
            .as_array()
            .ok_or_else(|| RpcError::MissingData("{{ transactions: [...] }}".to_string()))?;
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
        let request_endpoint = "/get_transaction_pool_hashes";

        let res = self.request(request_body, request_endpoint).await?;

        let blobs = if let Some(h) = res["tx_hashes"].as_array() {
            h
        } else {
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
            // We've gotta grab these in parts to avoid putting too much load on the RPC server, so
            // these are the start and end indexes of the hashes we're grabbing for now.
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
                + &serde_json::json!(hashes[starting_index..ending_index]
                    .iter()
                    .map(|x| hex::encode(x.as_bytes())) // Convert from monero::Hash to hex.
                    .collect::<Vec<String>>())
                .to_string()
                + "}";
            let request_endpoint = "/get_transactions";

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
                let tx_str = tx_json
                    .as_str()
                    .expect("failed to read transaction hex from json");
                let tx_hex = hex::decode(tx_str)?;
                let tx: monero::Transaction = deserialize(&tx_hex)?;
                transactions.push(tx);
            }
        }
        Ok(transactions)
    }

    pub async fn daemon_height(&self) -> Result<u64, RpcError> {
        let request_body = r#"{"jsonrpc":"2.0","id":"0","method":"get_block_count"}"#;
        let request_endpoint = "/json_rpc";

        let res = self.request(request_body, request_endpoint).await?;

        let count = res["result"]["count"]
            .as_u64()
            .ok_or_else(|| RpcError::MissingData("{{ result: {{ count: \"...\" }}".to_string()))?;

        Ok(count - 1)
    }

    async fn request(&self, body: &str, endpoint: &str) -> Result<serde_json::Value, RpcError> {
        let res = self
            .client
            .post(self.url.clone() + endpoint)
            .body(body.to_owned())
            .send()
            .await?;
        Ok(res.json::<serde_json::Value>().await?)
    }
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug)]
pub enum RpcError {
    Http(reqwest::Error),
    HexDecode(hex::FromHexError),
    Serialization(encode::Error),
    MissingData(String),
    DataType {
        found: serde_json::Value,
        expected: &'static str,
    },
}

impl From<reqwest::Error> for RpcError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

impl From<hex::FromHexError> for RpcError {
    fn from(e: hex::FromHexError) -> Self {
        Self::HexDecode(e)
    }
}

impl From<encode::Error> for RpcError {
    fn from(e: encode::Error) -> Self {
        Self::Serialization(e)
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcError::Http(e) => {
                write!(f, "http request failed: {}", e)
            }
            RpcError::HexDecode(e) => {
                write!(f, "hex decoding failed: {}", e)
            }
            RpcError::Serialization(e) => {
                write!(f, "(de)serialization failed: {}", e)
            }
            RpcError::MissingData(s) => {
                write!(
                    f,
                    "expected data was not present in RPC response, or was the wrong data type: {}",
                    s
                )
            }
            RpcError::DataType { found, expected } => {
                write!(
                    f,
                    "failed to interpret json value \"{}\" from RPC response as {}",
                    found, expected
                )
            }
        }
    }
}

impl Error for RpcError {}
