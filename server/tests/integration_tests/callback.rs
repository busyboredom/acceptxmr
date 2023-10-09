use std::{path::PathBuf, str::FromStr, time::Duration};

use acceptxmr::{storage::stores::Sqlite, PaymentGatewayBuilder};
use acceptxmr_server::{build_server, load_config, run_server, spawn_gateway, Config};
use hyper::http::Uri;
use log::{debug, info};
use serde_json::json;
use testing_utils::{init_logger, MockDaemon, PRIMARY_ADDRESS, PRIVATE_VIEW_KEY};

use crate::common::{CallbackListener, GatewayClient, MockNewInvoicePayload};

#[tokio::test]
async fn callback() {
    init_logger();
    let mock_daemon = MockDaemon::new_mock_daemon().await;

    let store = Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap();
    let payment_gateway = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        store,
    )
    // Faster scan rate so the update is received sooner.
    .scan_interval(Duration::from_millis(100))
    .daemon_url(mock_daemon.url(""))
    .seed(1)
    .build()
    .await
    .unwrap();
    info!("Payment gateway created.");

    let config = load_config(&PathBuf::from(Config::DEFAULT_PATH));
    let payment_gateway = spawn_gateway(payment_gateway, &config).await;

    let server = build_server(&config, payment_gateway).await;
    let address = server.internal_ipv4_address().unwrap();
    tokio::spawn(run_server(server));

    let mut client = GatewayClient::default();
    client.token = Some("supersecrettoken".to_string());
    client.url = Uri::from_str(&format!("https://{}:{}", address.ip(), address.port())).unwrap();

    let mut callback_listener = CallbackListener::init().await;
    let callback_url = callback_listener.url();

    let new_invoice_payload = MockNewInvoicePayload {
        callback: Some(callback_url.to_string()),
        ..Default::default()
    };
    let new_invoice_response = client
        .new_invoice(new_invoice_payload)
        .await
        .expect("failed to call `checkout` endpoint");
    debug!("Checkout response: {:?}", new_invoice_response);

    // Listen for callback from initial update.
    let callback = callback_listener
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for callback")
        .expect("channel to callback listener closed");

    assert_eq!(
        json!(callback),
        json!({
            "address": "82ZZhxB2dAtGwRQSSzvc9fUfM2oFWCUBUFJUAYDsureAB57RZEXm7fyZjwVXGyDGMA3wMtZjMSzECjfbkk5jYkA1SDmWWkx",
            "amount_paid": 0,
            "amount_requested": 2_234_345,
            "callback": format!("http://127.0.0.1:{}/", callback_listener.port()),
            "confirmations": None::<u64>,
            "confirmations_required": 2,
            "current_height": 2_477_657,
            "expiration_in": 20,
            "id": "AAAAAAAAAGEAAAAAACXOWQ",
            "order": "I am a test order",
            "uri": "monero:82ZZhxB2dAtGwRQSSzvc9fUfM2oFWCUBUFJUAYDsureAB57RZEXm7fyZjwVXGyDGMA3wMtZjMSzECjfbkk5jYkA1SDmWWkx?tx_amount=0.000002234345"
        })
    );
}

/// Fail the first callback and assert that it is retried.
#[tokio::test]
async fn callback_retry() {
    init_logger();
    let mock_daemon = MockDaemon::new_mock_daemon().await;

    let store = Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap();
    let payment_gateway = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        store,
    )
    // Faster scan rate so the update is received sooner.
    .scan_interval(Duration::from_millis(1000))
    .daemon_url(mock_daemon.url(""))
    .seed(1)
    .build()
    .await
    .unwrap();
    info!("Payment gateway created.");

    let config = load_config(&PathBuf::from(Config::DEFAULT_PATH));
    let payment_gateway = spawn_gateway(payment_gateway, &config).await;

    let server = build_server(&config, payment_gateway).await;
    info!("Built AcceptXMR server.");
    let address = server.internal_ipv4_address().unwrap();
    info!("Starting with internal address {address}");
    tokio::spawn(run_server(server));

    let mut client = GatewayClient::default();
    client.token = Some("supersecrettoken".to_string());
    client.url = Uri::from_str(&format!("https://{}:{}", address.ip(), address.port())).unwrap();

    info!("Starting callback listener.");
    let mut callback_listener = CallbackListener::init().await;
    info!("Started callback listener.");
    callback_listener.fail_one_callback().await;
    callback_listener.fail_one_callback().await;
    let callback_url = callback_listener.url();

    let new_invoice_payload = MockNewInvoicePayload {
        callback: Some(callback_url.to_string()),
        ..Default::default()
    };
    info!("Creating new invoice.");
    let new_invoice_response = client
        .new_invoice(new_invoice_payload)
        .await
        .expect("failed to call `checkout` endpoint");
    info!("Checkout response: {:?}", new_invoice_response);

    // This one will have been failed.
    callback_listener
        .recv_timeout(Duration::from_secs(120))
        .await
        .unwrap();

    // This one will also have been failed.
    callback_listener
        .recv_timeout(Duration::from_secs(120))
        .await
        .unwrap();

    // Now ensure it gets retried.
    let callback = callback_listener
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for callback")
        .expect("channel to callback listener closed");
    assert_eq!(callback.order, "I am a test order");
    assert_eq!(callback.uri, "monero:82ZZhxB2dAtGwRQSSzvc9fUfM2oFWCUBUFJUAYDsureAB57RZEXm7fyZjwVXGyDGMA3wMtZjMSzECjfbkk5jYkA1SDmWWkx?tx_amount=0.000002234345");
}
