use std::thread;

use qrcode::render::svg;
use qrcode::QrCode;
use tokio::fs;
use tokio::time;

use xmr_checkout::{BlockScannerBuilder, Payment};

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

    block_scanner.run();

    let payment = Payment::new(&payment_id, 1, 1, 99999999);
    let payment_updates = block_scanner.track_payment(payment);
    let mut paid = false;
    while paid == false {
        thread::sleep(time::Duration::from_millis(5000));
        for updated_payment in payment_updates.try_iter() {
            if updated_payment.paid_amount >= updated_payment.expected_amount {
                paid = true;
            }
        }
    }
}
