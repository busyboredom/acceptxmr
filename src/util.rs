use std::collections::HashMap;

use monero::blockdata::transaction::{ExtraField, SubField};
use monero::consensus::deserialize;
use monero::cryptonote::hash::keccak_256;
use monero::util::address::PaymentId;

use crate::Payment;

pub fn scan_transactions(
    viewpair: &monero::ViewPair,
    payments: &mut HashMap<PaymentId, Payment>,
    transactions: Vec<monero::Transaction>,
) {
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
                    let key = keccak_256(&key_bytes);

                    // The first byte of the nonce is not part of the encrypted payment ID.
                    let mut id_bytes = nonce_bytes.clone()[1..9].to_vec();

                    // Decrypt the payment ID by XORing it with the key.
                    id_bytes
                        .iter_mut()
                        .zip(key.iter())
                        .for_each(|(x1, x2)| *x1 ^= *x2);

                    payment_id = PaymentId::from_slice(&id_bytes);
                    println!("Payment ID: {}", hex::encode(&payment_id.as_bytes()))
                }
            }
        }

        // If this payment is being tracked, update the amount paid.
        if let Some(payment) = payments.get_mut(&payment_id) {
            payment.paid_amount += owned_outputs[0]
                .amount()
                .expect("Failed to unblind transaction amount");
        }
    }
}

pub async fn get_block(url: &str, height: u64) -> Result<monero::Block, reqwest::Error> {
    let client = reqwest::Client::new();

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
    println!(
        "Block request status: {}",
        block_json["result"]["status"].as_str().unwrap()
    );

    let block_blob = block_json["result"]["blob"]
        .as_str()
        .expect("Failed to read block blob from json_rpc");

    let block_bytes =
        hex::decode(block_blob).expect("Failed to decode block blob from hex to bytes");

    let block = deserialize(&block_bytes).expect("Failed to deserialize block blob");

    Ok(block)
}

pub async fn get_block_transactions(
    url: &str,
    block: monero::Block,
) -> Result<Vec<monero::Transaction>, reqwest::Error> {
    // Get block transactions in sets of 100 or less (the restriced RPC maximum).
    let client = reqwest::Client::new();
    let mut transactions = vec![];
    let transaction_hashes = block.tx_hashes;
    println!("Transactions to fetch: {}", transaction_hashes.len());
    for i in 0..(transaction_hashes.len() / 100 + 1) {
        // Start and end indexes of the hashes we're grabbing for now.
        let starting_index: usize = i * 100;
        let ending_index: usize = std::cmp::min(100 * (i + 1), transaction_hashes.len());
        println!("Transactions requested: {}", ending_index - starting_index);

        // Build a json containing the hashes of the transactions we want.
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
        println!(
            "Transaction request status: {}",
            res["status"]
                .as_str()
                .expect("Failed to read status from json request")
        );

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

    println!("Transactions fetched: {}", transactions.len());

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
