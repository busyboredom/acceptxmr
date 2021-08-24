use std::env;

use log::info;
use qrcode::render::svg;
use qrcode::QrCode;
use tokio::fs;
use actix_web::{get, web, App, HttpServer, Responder};
use actix_files;

use acceptxmr::{BlockScannerBuilder, Payment};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env::set_var("RUST_LOG", "debug,mio=debug,want=debug,reqwest=info");
    env_logger::init();

    // Prepare Viewkey.
    let mut viewkey_string = include_str!("../../../secrets/xmr_private_viewkey.txt").to_string();
    viewkey_string.pop();

    let xmr_daemon_url = "http://busyboredom.com:18081";
    let mut block_scanner = BlockScannerBuilder::new()
        .daemon_url(xmr_daemon_url)
        .private_viewkey(&viewkey_string)
        .public_spendkey("dd4c491d53ad6b46cda01ed6cb9bac57615d9eac8d5e4dd1c0363ac8dfd420a7")
        .scan_rate(1000)
        .build();

    // Get a new integrated address, and the payment ID contained in it.
    let (address, payment_id) = block_scanner.new_integrated_address();
    info!("Payment ID generated: {}", payment_id);

    // Render a QR code for the new address.
    let qr = QrCode::new(address).unwrap();
    let image = qr.render::<svg::Color>().module_dimensions(1, 1).build();

    // Save the QR code image.
    fs::write("static/qrcode.svg", image)
        .await
        .expect("Unable to write QR Code image to file");

    let current_height = block_scanner.get_current_height().await.unwrap();
    block_scanner.run(10, current_height - 10);

    let payment = Payment::new(&payment_id, 1, 1, 99999999);
    let payment_updates = block_scanner.track_payment(payment);

    HttpServer::new(|| App::new().service(actix_files::Files::new("/", "./static")))
        .bind("127.0.0.1:8080")?
        .run()
        .await
}