use log::trace;
use monero::consensus::deserialize;

use crate::Error;

pub async fn get_block(url: &str, height: u64) -> Result<(monero::Hash, monero::Block), Error> {
    let client = reqwest::Client::new();

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

    let block_id_hex = block_json["result"]["block_header"]["hash"]
        .as_str()
        .unwrap();
    let block_id = monero::Hash::from_slice(&hex::decode(block_id_hex).unwrap());

    let block_blob = block_json["result"]["blob"]
        .as_str()
        .expect("Failed to read block blob from json_rpc");

    let block_bytes =
        hex::decode(block_blob).expect("Failed to decode block blob from hex to bytes");

    let block = deserialize(&block_bytes).expect("Failed to deserialize block blob");

    Ok((block_id, block))
}

pub async fn get_block_transactions(
    url: &str,
    block: &monero::Block,
) -> Result<Vec<monero::Transaction>, Error> {
    // Get block transactions in sets of 100 or less (the restriced RPC maximum).
    // TODO: Get them concurrently.
    let client = reqwest::Client::new();
    let mut transactions = vec![];
    let transaction_hashes = &block.tx_hashes;
    for i in 0..(transaction_hashes.len() / 100 + 1) {
        // Start and end indexes of the hashes we're grabbing for now.
        let starting_index: usize = i * 100;
        let ending_index: usize = std::cmp::min(100 * (i + 1), transaction_hashes.len());

        // Build a json containing the hashes of the transactions we want.
        trace!("Requesting {} transactions.", transaction_hashes.len());
        let request_body = r#"{"txs_hashes":"#.to_owned()
            + &serde_json::json!(transaction_hashes[starting_index..ending_index]
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
                    .expect("Failed to read transaction hex from json");
                let tx_hex =
                    hex::decode(tx_str).expect("Failed to decode transaction fron hex to bytes");
                let tx = deserialize(&tx_hex).expect("Failed to deserialize transaction");
                transactions.push(tx);
            }
        }
    }

    Ok(transactions)
}

pub async fn get_txpool(url: &str) -> Result<Vec<monero::Transaction>, Error> {
    let client = reqwest::Client::new();

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
            let tx_hex = hex::decode(blob["tx_blob"].as_str().unwrap()).unwrap();
            let tx = deserialize(&tx_hex).unwrap();
            transactions.push(tx);
        }
    };

    Ok(transactions)
}

pub async fn get_current_height(url: &str) -> Result<u64, Error> {
    let client = reqwest::Client::new();

    let request_body = r#"{"jsonrpc":"2.0","id":"0","method":"get_block_count"}"#;
    let res = client
        .post(url.to_owned() + "/json_rpc")
        .body(request_body)
        .send()
        .await?;
    let res = res.json::<serde_json::Value>().await?;

    let height = res["result"]["count"].as_u64().unwrap() - 1;

    Ok(height)
}
