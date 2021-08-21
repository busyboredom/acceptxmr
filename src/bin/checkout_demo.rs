use std::collections::HashMap;

use monero::util::address::PaymentId;
use qrcode::render::svg;
use qrcode::QrCode;
use tokio::fs;
use tokio::time;

use xmr_checkout::{BlockScanner, BlockScannerBuilder, Payment};

#[tokio::main]
async fn main() {
    // Prepare Viewkey.
    let mut viewkey_string = include_str!("../../../secrets/xmr_private_viewkey.txt").to_string();
    viewkey_string.pop();

    let mut block_scanner = BlockScannerBuilder::new()
        .daemon_url("http://busyboredom.com:18081")
        .private_viewkey(&viewkey_string)
        .public_spendkey("dd4c491d53ad6b46cda01ed6cb9bac57615d9eac8d5e4dd1c0363ac8dfd420a7")
        .scan_rate(1000)
        .build();

    // Get a new integrated address, and the payment ID contained in it.
    let (address, payment_id) = block_scanner.new_integrated_address();
    println!("Payment ID: {}", payment_id);

    // Render a QR code for the new address.
    let qr = QrCode::new(address).unwrap();
    let image = qr.render::<svg::Color>().min_dimensions(200, 200).build();

    // Save the QR code image.
    fs::write("qrcode.svg", image)
        .await
        .expect("Unable to wtire QR Code image to file");

    // Below this is old test code --------------------------------------------------------

    let mut blockscan_interval = time::interval(time::Duration::from_secs(1));
    loop {
        blockscan_interval.tick().await;
        let current_height = get_current_height().await.unwrap();
        let current_height = 2431442;
        println!("Current Block: {}", current_height);
        let amount = scan_block_transactions(current_height, &mut block_scanner)
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
    block_scanner: &mut BlockScanner,
) -> Result<u64, reqwest::Error> {
    let block = block_scanner.get_block(height).await?;
    let transactions = block_scanner.get_block_transactions(block).await?;
    println!("Transactions: {}", transactions.len());

    // TODO: write a function for blockscanner that lets me insert this payment.
    let payment_id = PaymentId::from_slice(&hex::decode("33d2a0f45130c85b").unwrap());
    let payment = Payment {
        payment_id: payment_id,
        expected_amount: 1,
        paid_amount: 0,
        confirmations_required: 1,
        confirmations_recieved: 0,
        expiration_block: 10,
    };
    block_scanner.scan_transactions(transactions).await;

    let amount = payment.paid_amount;

    println!("Amount: {}", monero::Amount::from_pico(amount).as_xmr());

    Ok(amount)
}
