use std::convert::TryInto;
use std::str::FromStr;

use monero::blockdata::transaction;
use monero::consensus::encode;
use monero::cryptonote::hash::keccak_256;
use monero::util::address::PaymentId;
use monero::util::{address, key};
use qrcode::render::svg;
use qrcode::QrCode;
use serde_json::{json, Value};
use tokio::fs;
use tokio::time;

use xmr_checkout::{BlockScanner, BlockScannerBuilder};

#[tokio::main]
async fn main() {
    // Prepare Viewkey.
    let mut viewkey_string = include_str!("../../../secrets/xmr_private_viewkey.txt").to_string();
    viewkey_string.pop();
    let private_viewkey = key::PrivateKey::from_str(&viewkey_string).unwrap();

    // Prepare Spendkey.
    let public_spendkey = key::PublicKey::from_str(
        "dd4c491d53ad6b46cda01ed6cb9bac57615d9eac8d5e4dd1c0363ac8dfd420a7",
    )
    .unwrap();

    let block_scanner = BlockScannerBuilder::new()
        .daemon_url("https://busyboredom.com:18081")
        .private_viewkey(&viewkey_string)
        .public_spendkey("dd4c491d53ad6b46cda01ed6cb9bac57615d9eac8d5e4dd1c0363ac8dfd420a7")
        .scan_rate(1000)
        .build();

    // Get a new integrated address, and the payment ID contained in it.
    let (address, payment_id) = block_scanner.new_integrated_address();
    println!("Payment ID: {:#?}", payment_id);

    // Render a QR code for the new address.
    let qr = QrCode::new(address).unwrap();
    let image = qr.render::<svg::Color>().min_dimensions(200, 200).build();

    // Save the QR code image.
    fs::write("qrcode.svg", image)
        .await
        .expect("Unable to wtire QR Code image to file");

    // Below this is old test code --------------------------------------------------------

    // Combine into keypair.
    let viewpair = key::ViewPair {
        view: private_viewkey,
        spend: public_spendkey,
    };

    let mut blockscan_interval = time::interval(time::Duration::from_secs(1));
    loop {
        blockscan_interval.tick().await;
        let current_height = get_current_height().await.unwrap();
        println!("{:?}", current_height);
        let amount = scan_block_transactions(current_height, &viewpair)
            .await
            .unwrap();
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

async fn scan_block_transactions(
    height: u64,
    viewpair: &key::ViewPair,
) -> Result<u64, reqwest::Error> {
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

    // Get block transactions in sets of 100 or less (the restriced RPC maximum).
    let mut transaction_hexes = vec![];
    let mut transaction_hashes = vec![];
    if let Some(tx_hashes) = res["result"]["tx_hashes"].as_array() {
        transaction_hashes = tx_hashes.to_vec();
    }
    for i in 0..(transaction_hashes.len() / 100 + 1) {
        let starting_index: usize = i * 100;
        let ending_index: usize = std::cmp::min(100 * (i + 1), transaction_hashes.len());
        let request_body = r#"{"txs_hashes":"#.to_owned()
            + &json!(transaction_hashes[starting_index..ending_index]).to_string()
            + "}";
        let res = client
            .post("http://busyboredom.com:18081/get_transactions")
            .body(request_body)
            .send()
            .await?;

        let res = res.json::<serde_json::Value>().await?;

        // Add these transactions to the total list.
        if let Some(hexes) = res["txs_as_hex"].as_array() {
            transaction_hexes.append(&mut hexes.to_vec());
        }
    }

    let mut total_amount: u64 = 0;
    for tx_hex_json in transaction_hexes {
        let tx_hex_str = tx_hex_json.as_str().unwrap();
        let tx_hex = hex::decode(tx_hex_str).unwrap();
        let tx: transaction::Transaction = encode::deserialize(&tx_hex).unwrap();
        let owned_outputs = tx.check_outputs(&viewpair, 0..2, 0..3).unwrap();
        for output in &owned_outputs {
            total_amount += output
                .amount()
                .expect("Failed to read amount from owned output");
        }

        // Generate and display the SubFields (the parsed "extra" section) if applicable.
        let payment_id: Option<address::PaymentId> = None;
        if owned_outputs.len() == 1 {
            let tx_extra = &tx.prefix().extra;
            let transaction::ExtraField(subfields) = tx_extra;
            for subfield in subfields {
                println!("SubField: {:#?}", subfield);
                if let transaction::SubField::Nonce(bytes) = subfield {
                    // THIS STUFF DOESN'T WORK YET
                    let shared_secret = tx.tx_pubkey().unwrap() * &(viewpair.view * 8u8);
                    let mut key_bytes = shared_secret.as_bytes().to_vec();
                    key_bytes.append(&mut hex::decode("8d").unwrap());
                    let key = keccak_256(&key_bytes);
                    let mut id_bytes = bytes.clone()[0..8].to_vec();
                    id_bytes
                        .iter_mut()
                        .zip(key.iter())
                        .for_each(|(x1, x2)| *x1 ^= *x2);
                    let payment_id = Some(address::PaymentId::from_slice(&id_bytes));
                    println!("Payment ID: {:#?}", &id_bytes)
                }
            }
        }
    }

    println!("{}", monero::Amount::from_pico(total_amount).as_xmr());

    Ok(total_amount)
}
