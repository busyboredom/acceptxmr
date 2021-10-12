// TODO: Make this a struct and name it Rpc.

use std::fmt;
use std::future::Future;
use std::{collections::HashSet, error::Error};

use log::{error, trace};
use monero::consensus::{deserialize, encode};
use tokio::{join, time};

pub async fn get_block(url: &str, height: u64) -> Result<(monero::Hash, monero::Block), RpcError> {
    let client = reqwest::ClientBuilder::new()
        .connect_timeout(time::Duration::from_millis(2000))
        .build()?;

    trace!("Requesting block {}", height);
    let request_body = r#"{"jsonrpc":"2.0","id":"0","method":"get_block","params":{"height":"#
        .to_owned()
        + &height.to_string()
        + "}}";
    let res = client
        .post(url.to_owned() + "/json_rpc")
        .body(request_body)
        .send()
        .await?;

    let block_json = res.json::<serde_json::Value>().await?;

    let block_id_hex = match block_json["result"]["block_header"]["hash"].as_str() {
        Some(s) => s,
        None => return Err(RpcError::MissingData),
    };
    let block_id = monero::Hash::from_slice(&hex::decode(block_id_hex)?);

    let block_blob = block_json["result"]["blob"]
        .as_str()
        .expect("failed to read block blob from json_rpc");

    let block_bytes =
        hex::decode(block_blob).expect("failed to decode block blob from hex to bytes");

    let block = deserialize(&block_bytes).expect("failed to deserialize block blob");

    Ok((block_id, block))
}

pub async fn get_block_transactions(
    url: &str,
    block: &monero::Block,
) -> Result<Vec<monero::Transaction>, RpcError> {
    // Get block transactions in sets of 100 or less (the restricted RPC maximum).
    // TODO: Get them concurrently.
    let transaction_hashes = &block.tx_hashes;
    get_transactions_by_hashes(url, transaction_hashes).await
}

pub async fn get_txpool(url: &str) -> Result<Vec<monero::Transaction>, RpcError> {
    let client = reqwest::ClientBuilder::new()
        .connect_timeout(time::Duration::from_millis(2000))
        .build()?;

    trace!("Requesting txpool");
    // TODO: Consider using json_rpc method for this.
    let res = client
        .post(url.to_owned() + "/get_transaction_pool")
        .body("")
        .send()
        .await?;
    let res = res.json::<serde_json::Value>().await?;

    let transaction_blobs = res["transactions"].as_array();
    let mut transactions = Vec::new();
    if let Some(blobs) = transaction_blobs {
        for blob in blobs {
            let tx_str = match blob["tx_blob"].as_str() {
                Some(s) => s,
                None => continue,
            };
            let tx_hex = hex::decode(tx_str)?;
            let tx = deserialize(&tx_hex)?;
            transactions.push(tx);
        }
    };

    Ok(transactions)
}

pub async fn get_txpool_hashes(url: &str) -> Result<HashSet<monero::Hash>, RpcError> {
    let client = reqwest::ClientBuilder::new()
        .connect_timeout(time::Duration::from_millis(2000))
        .build()?;

    trace!("Requesting txpool hashes");
    let res = client
        .post(url.to_owned() + "/get_transaction_pool_hashes")
        .body("")
        .send()
        .await?;
    let res = res.json::<serde_json::Value>().await?;

    let transaction_hashes_blobs = res["tx_hashes"].as_array();
    let mut transactions = HashSet::new();
    if let Some(blobs) = transaction_hashes_blobs {
        for blob in blobs {
            let tx_hash_str = match blob.as_str() {
                Some(hs) => hs,
                None => continue,
            };
            let tx_hash_hex = hex::decode(tx_hash_str)?;
            let tx_hash = deserialize(&tx_hash_hex)?;
            transactions.insert(tx_hash);
        }
    };

    Ok(transactions)
}

pub async fn get_transactions_by_hashes(
    url: &str,
    hashes: &[monero::Hash],
) -> Result<Vec<monero::Transaction>, RpcError> {
    let client = reqwest::ClientBuilder::new()
        .connect_timeout(time::Duration::from_millis(2000))
        .build()?;
    let mut transactions = Vec::new();
    for i in 0..(hashes.len() / 100 + 1) {
        // We've gotta grab these in parts to avoid putting too much load on the RPC server, so
        // these are the start and end indexes of the hashes we're grabbing for now.
        let starting_index: usize = i * 100;
        let ending_index: usize = std::cmp::min(100 * (i + 1), hashes.len());

        // Build a json containing the hashes of the transactions we want.
        trace!("Requesting {} transactions", hashes.len());
        let request_body = r#"{"txs_hashes":"#.to_owned()
            + &serde_json::json!(hashes[starting_index..ending_index]
                .iter()
                .map(|x| hex::encode(x.as_bytes())) // Convert from monero::Hash to hex.
                .collect::<Vec<String>>())
            .to_string()
            + "}";
        let res = client
            .post(url.to_owned() + "/get_transactions")
            .body(request_body)
            .send()
            .await?;

        let res = res.json::<serde_json::Value>().await?;

        // Add these transactions to the total list.
        if let Some(hexes) = res["txs_as_hex"].as_array() {
            for tx_json in hexes {
                let tx_str = tx_json
                    .as_str()
                    .expect("failed to read transaction hex from json");
                let tx_hex = hex::decode(tx_str)?;
                let tx = deserialize(&tx_hex)?;
                transactions.push(tx);
            }
        }
    }

    Ok(transactions)
}

pub async fn get_daemon_height(url: &str) -> Result<u64, RpcError> {
    let client = reqwest::ClientBuilder::new()
        .connect_timeout(time::Duration::from_millis(2000))
        .build()?;

    let request_body = r#"{"jsonrpc":"2.0","id":"0","method":"get_block_count"}"#;
    let res = client
        .post(url.to_owned() + "/json_rpc")
        .body(request_body)
        .send()
        .await?;
    let res = res.json::<serde_json::Value>().await?;

    let count = match res["result"]["count"].as_u64() {
        Some(c) => c,
        None => return Err(RpcError::MissingData),
    };
    let height = count - 1;

    Ok(height)
}

pub async fn retry<'a, T, E, Fut>(url: &'a str, retry_millis: u64, f: impl Fn(&'a str) -> Fut) -> T
where
    Fut: Future<Output = Result<T, E>>,
    E: Error,
{
    let mut retry_interval = time::interval(time::Duration::from_millis(retry_millis));
    loop {
        let (t_or_err, _) = join!(f(url), retry_interval.tick());
        match t_or_err {
            Ok(t) => return t,
            Err(e) => {
                error!("{}. Retrying in {} ms.", e, retry_millis);
                continue;
            }
        }
    }
}

pub async fn retry_vec<'a, T, E, X, Fut>(
    url: &'a str,
    v: &'a [X],
    retry_millis: u64,
    f: impl Fn(&'a str, &'a [X]) -> Fut,
) -> T
where
    Fut: Future<Output = Result<T, E>>,
    E: Error,
{
    let mut retry_interval = time::interval(time::Duration::from_millis(retry_millis));
    loop {
        let (t_or_err, _) = join!(f(url, v), retry_interval.tick());
        match t_or_err {
            Ok(t) => return t,
            Err(e) => {
                error!("{}. Retrying in {} ms.", e, retry_millis);
                continue;
            }
        }
    }
}

#[derive(Debug)]
pub enum RpcError {
    Http(reqwest::Error),
    HexDecode(hex::FromHexError),
    Serialization(encode::Error),
    MissingData,
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
            RpcError::Http(reqwest_error) => {
                write!(f, "http request error: {}", reqwest_error)
            }
            RpcError::HexDecode(hex_error) => {
                write!(f, "hex decoding error: {}", hex_error)
            }
            RpcError::Serialization(ser_error) => {
                write!(f, "serialization error: {}", ser_error)
            }
            RpcError::MissingData => {
                write!(f, "expected data was not present in RPC response")
            }
        }
    }
}

impl Error for RpcError {}
