use std::{path::PathBuf, str::FromStr, time::Duration};

use acceptxmr::{storage::stores::Sqlite, PaymentGatewayBuilder};
use acceptxmr_server::{
    api::types::invoice_id::Base64InvoiceId, build_server, load_config, run_server, spawn_gateway,
    Config,
};
use futures::StreamExt;
use http_body_util::BodyExt;
use hyper::http::Uri;
use log::{debug, info};
use serde_json::{json, Value};
use testing_utils::{init_logger, MockDaemon, PRIMARY_ADDRESS, PRIVATE_VIEW_KEY};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite;

use crate::common::{CallbackListener, GatewayClient, MockInvoiceIdPayload, MockNewInvoicePayload};

#[tokio::test]
async fn websocket() {
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
    let internal_address = server.internal_ipv4_address().unwrap();
    let external_address = server.external_ipv4_address().unwrap();
    tokio::spawn(run_server(server));

    let mut internal_client = GatewayClient::default();
    internal_client.token = Some("supersecrettoken".to_string());
    internal_client.url = Uri::from_str(&format!(
        "https://{}:{}",
        internal_address.ip(),
        internal_address.port()
    ))
    .unwrap();

    let mut callback_listener = CallbackListener::init().await;

    let base64_invoice_id = new_invoice(&internal_client, &callback_listener).await;

    // Listen for callback from initial update.
    let callback = callback_listener
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for callback")
        .expect("channel to callback listener closed");
    assert_eq!(callback.order, "I am a test order");
    assert_eq!(callback.uri, "monero:82ZZhxB2dAtGwRQSSzvc9fUfM2oFWCUBUFJUAYDsureAB57RZEXm7fyZjwVXGyDGMA3wMtZjMSzECjfbkk5jYkA1SDmWWkx?tx_amount=0.000002234345");

    let mut external_client = GatewayClient::default();
    external_client.url = Uri::from_str(&format!(
        "http://{}:{}",
        external_address.ip(),
        external_address.port()
    ))
    .unwrap();

    // Add transfer to txpool.
    let txpool_hashes_mock = mock_daemon.mock_txpool_hashes(
        "../testing-utils/rpc_resources/txpools/hashes_with_payment_account_0.json",
    );
    let txpool_transactions_mock = mock_daemon.mock_txpool_transactions(
        "../testing-utils/rpc_resources/transactions/hashes_with_payment_account_0.json",
        "../testing-utils/rpc_resources/transactions/txs_with_payment_account_0.json",
    );

    let mut websocket = external_client
        .subscribe_to_websocket(base64_invoice_id.clone())
        .await
        .expect("failed to call `/invoice/ws` endpoint");

    let msg = match timeout(Duration::from_secs(120), websocket.next())
        .await
        .unwrap()
        .expect("websocket closed unexpectedly")
        .unwrap()
    {
        tungstenite::Message::Text(msg) => msg,
        other => panic!("Unexpected message type: {other}"),
    };
    debug!("{msg}");

    assert!(txpool_hashes_mock.hits() > 0);
    assert!(txpool_transactions_mock.hits() > 0);

    assert_eq!(
        Value::from_str(&msg).unwrap(),
        json!({
            "address": "82ZZhxB2dAtGwRQSSzvc9fUfM2oFWCUBUFJUAYDsureAB57RZEXm7fyZjwVXGyDGMA3wMtZjMSzECjfbkk5jYkA1SDmWWkx",
            "amount_paid": 1_468_383_460,
            "amount_requested": 2_234_345,
            "callback": format!("http://127.0.0.1:{}/", callback_listener.port()),
            "confirmations": 0,
            "confirmations_required": 2,
            "current_height": 2_477_657,
            "expiration_in": 20,
            "id": "AAAAAAAAAGEAAAAAACXOWQ",
            "order": "I am a test order",
            "uri": "monero:82ZZhxB2dAtGwRQSSzvc9fUfM2oFWCUBUFJUAYDsureAB57RZEXm7fyZjwVXGyDGMA3wMtZjMSzECjfbkk5jYkA1SDmWWkx?tx_amount=0.0"
        })
    );
}

async fn new_invoice(
    client: &GatewayClient,
    callback_listener: &CallbackListener,
) -> Base64InvoiceId {
    let new_invoice_payload = MockNewInvoicePayload {
        callback: Some(callback_listener.url().to_string()),
        ..Default::default()
    };
    let new_invoice_response = client
        .new_invoice(new_invoice_payload)
        .await
        .expect("failed to call `checkout` endpoint");
    debug!("Checkout response: {:?}", new_invoice_response);
    let MockInvoiceIdPayload {
        invoice_id: base64_invoice_id,
    } = serde_json::from_slice(
        &new_invoice_response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
    )
    .unwrap();

    base64_invoice_id
}
