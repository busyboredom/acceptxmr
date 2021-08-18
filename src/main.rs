use std::str::FromStr;

use monero::util::{key, address};
use monero::blockdata::transaction;
use monero::consensus::encode;
use tokio::time;

#[tokio::main]
async fn main() {
    println!("Hello, world!");

    // Prepare Viewkey.
    let mut viewkey_string = include_str!("../../secrets/xmr_private_viewkey.txt").to_string();
    viewkey_string.pop();
    let private_viewkey = key::PrivateKey::from_str(&viewkey_string).unwrap();

    // Prepare Spendkey.
    let public_spendkey = key::PublicKey::from_str("dd4c491d53ad6b46cda01ed6cb9bac57615d9eac8d5e4dd1c0363ac8dfd420a7").unwrap();

    // Combine into keypair.
    let view_pair = key::ViewPair {
        view: private_viewkey,
        spend: public_spendkey

    };

    let mut blockscan_interval = time::interval(time::Duration::from_secs(1));
    loop {
        blockscan_interval.tick().await;
        let current_height = get_current_height().await.unwrap();
        println!("{:?}", current_height);
        let amount = scan_block_transactions(2429747, &view_pair).await.unwrap();
        if amount != 0 {
            break;
        }
    }
}

async fn get_current_height() -> Result<u64, reqwest::Error> {
    let client = reqwest::Client::new();

    let request_body = r#"{"jsonrpc":"2.0","id":"0","method":"get_block_count"}"#;
    let res = client
        .post("http://busyboredom.com:18081/json_rpc")
        .body(request_body)
        .send()
        .await?;
    let res = res.json::<serde_json::Value>().await?;
    
    let height = res["result"]["count"].as_u64().unwrap() - 1;

    Ok(height)
}

async fn scan_block_transactions(height: u64, view_pair: &key::ViewPair) -> Result<u64, reqwest::Error> {
    let client = reqwest::Client::new();

    let request_body = r#"{"jsonrpc":"2.0","id":"0","method":"get_block","params":{"height":"#
        .to_owned()
        + &height.to_string()
        + "}}";
    let res = client
        .post("http://busyboredom.com:18081/json_rpc")
        .body(request_body)
        .send()
        .await?;

    let res = res.json::<serde_json::Value>().await?;

    let request_body = r#"{"txs_hashes":"#.to_owned() + &res["result"]["tx_hashes"].to_string() + "}";
    let res = client
        .post("http://busyboredom.com:18081/get_transactions")
        .body(request_body)
        .send()
        .await?;

    let res = res.json::<serde_json::Value>().await?;

    let mut transaction_hexes = &vec![];
    if let Some(hexes) = res["txs_as_hex"].as_array() {
        transaction_hexes = hexes;
    }
    println!("Transactions in block: {}", transaction_hexes.len());
    let mut total_amount: u64 = 0;
    for tx_hex_json in transaction_hexes {
        let tx_hex_str = tx_hex_json.as_str().unwrap();
        let tx_hex = hex::decode(tx_hex_str).unwrap();
        let tx: transaction::Transaction = encode::deserialize(&tx_hex).unwrap();
        let owned_outputs = tx.check_outputs(&view_pair, 0..2, 0..3).unwrap();
        for output in owned_outputs {
            total_amount += output.amount().expect("Failed to read amount from owned output");
        }
    }

    println!("{}.{:012}", total_amount/1_000_000_000_000, total_amount%1_000_000_000_000);

    Ok(total_amount)
}

struct Payment {
    payment_id: address::PaymentId,
    expected_amount: u64,
    paid_amount: u64,
    confirmations_required: u64,
    confirmations_recieved: u64,
    expiration_block: u64
}