#![warn(clippy::pedantic)]

use acceptxmr::{storage::stores::Sqlite, PaymentGateway, PaymentGatewayBuilder};
use log::{error, info, LevelFilter};

#[tokio::main]
async fn main() {
    env_logger::builder()
        .filter_level(LevelFilter::Warn)
        .filter_module("acceptxmr", log::LevelFilter::Debug)
        .filter_module("persistence", log::LevelFilter::Trace)
        .init();

    // The private view key should be stored securely outside of the git repository.
    // It is hardcoded here for demonstration purposes only.
    let private_view_key = "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
    // No need to keep the primary address secret.
    let primary_address = "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";

    // Use an Sqlite database for persistent storage of pending invoices.
    let store = Sqlite::new("examples/persistence/AcceptXMR_DB", "invoices").unwrap();

    let payment_gateway = PaymentGatewayBuilder::new(
        private_view_key.to_string(),
        primary_address.to_string(),
        store,
    )
    .daemon_url("http://node.sethforprivacy.com:18089".to_string())
    .build()
    .unwrap();

    info!("Payment gateway created.");

    // Any invoices created with this payment gateway will now be stored
    // persistently in your Sqlite database.
    let invoice_id = payment_gateway
        .new_invoice(1000, 2, 5, "Demo invoice".to_string())
        .unwrap();
    let invoice = payment_gateway
        .get_invoice(invoice_id)
        .unwrap()
        .expect("invoice not found");

    info!(
        "Invoice retrieved from Sqlite database. Address: \n\n{}\n",
        invoice.address()
    );

    // Oh no, your server lost power!
    power_failure(payment_gateway);

    // Reconstruct the gateway...
    let store = Sqlite::new("examples/persistence/AcceptXMR_DB", "invoices").unwrap();
    let payment_gateway = PaymentGatewayBuilder::new(
        private_view_key.to_string(),
        primary_address.to_string(),
        store,
    )
    .daemon_url("http://node.sethforprivacy.com:18089".to_string())
    .build()
    .unwrap();

    // The invoice is still there!
    let invoice = payment_gateway
        .get_invoice(invoice_id)
        .unwrap()
        .expect("invoice not found");

    info!(
        "Invoice retrieved from Sqlite database. Address: \n\n{}\n",
        invoice.address()
    );
}

// An imaginary power failure for purposes of this example.
fn power_failure(payment_gateway: PaymentGateway<Sqlite>) {
    error!("Oh no, we lost power!");
    drop(payment_gateway);
}
