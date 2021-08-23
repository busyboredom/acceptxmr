use std::collections::HashMap;

use log::trace;
use monero::blockdata::transaction::{ExtraField, SubField};
use monero::consensus::deserialize;
use monero::util::address::PaymentId;

use crate::Payment;

pub fn scan_transactions(
    viewpair: &monero::ViewPair,
    payments: &HashMap<PaymentId, Payment>,
    transactions: Vec<monero::Transaction>,
) -> HashMap<PaymentId, u64> {
    let mut amounts_recieved = HashMap::new();
    for tx in transactions {
        let mut payment_id = PaymentId::zero();

        // Get owned outputs.
        let owned_outputs = tx.check_outputs(viewpair, 0..1, 0..1).unwrap();

        // Generate and display the SubFields (the parsed "extra" section) if applicable.
        if owned_outputs.len() == 1 {
            // Payments to integrated addresses only ever have one output.

            // Get transaction's "extra" section.
            let tx_extra = &tx.prefix().extra;

            // Get vec of subfields from transaction's "extra" section.
            let ExtraField(subfields) = tx_extra;

            for subfield in subfields {
                if let SubField::Nonce(nonce_bytes) = subfield {
                    // Shared secret can be retrieved as a combination of tx public key and your private view key.
                    let shared_secret = tx.tx_pubkey().unwrap() * &(viewpair.view * 8u8);

                    // The payment ID decryption key is a hash of the shared secret.
                    let mut key_bytes = shared_secret.as_bytes().to_vec();
                    key_bytes.append(&mut hex::decode("8d").unwrap());
                    let key = monero::Hash::hash(&key_bytes);

                    // The first byte of the nonce is not part of the encrypted payment ID.
                    let mut id_bytes = nonce_bytes.clone()[1..9].to_vec();

                    // Decrypt the payment ID by XORing it with the key.
                    id_bytes
                        .iter_mut()
                        .zip(key.as_bytes().iter())
                        .for_each(|(x1, x2)| *x1 ^= *x2);

                    payment_id = PaymentId::from_slice(&id_bytes);
                }
            }
        }

        // If this payment is being tracked, add the amount and payment ID to the result set.
        if payments.contains_key(&payment_id) {
            let amount = owned_outputs[0]
                .amount()
                .expect("Failed to unblind transaction amount");
            *amounts_recieved.entry(payment_id).or_insert(0) += amount;
        }
    }

    amounts_recieved
}

pub async fn get_block(
    url: &str,
    height: u64,
) -> Result<(monero::Hash, monero::Block), reqwest::Error> {
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
) -> Result<Vec<monero::Transaction>, reqwest::Error> {
    // Get block transactions in sets of 100 or less (the restriced RPC maximum).
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

pub async fn get_txpool(url: &str) -> Result<Vec<monero::Transaction>, reqwest::Error> {
    let client = reqwest::Client::new();

    trace!("Requesting txpool");
    let res = client
        .post(url.to_owned() + "/get_transaction_pool")
        .body("")
        .send()
        .await?;
    let res = res.json::<serde_json::Value>().await?;

    let transaction_blobs = res["transactions"].as_array().unwrap();
    let transactions = transaction_blobs
        .iter()
        .map(|x| deserialize(&hex::decode(x["tx_blob"].as_str().unwrap()).unwrap()).unwrap())
        .collect();

    Ok(transactions)
}

pub async fn get_current_height(url: &str) -> Result<u64, reqwest::Error> {
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
