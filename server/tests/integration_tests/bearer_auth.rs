use std::{path::PathBuf, str::FromStr};

use acceptxmr::{storage::stores::Sqlite, PaymentGatewayBuilder};
use acceptxmr_server::{build_server, load_config, run_server, Config};
use hyper::{http::Uri, StatusCode};
use log::{debug, info};
use test_case::test_case;
use testing_utils::{init_logger, MockDaemon, PRIMARY_ADDRESS, PRIVATE_VIEW_KEY};

use crate::common::{GatewayClient, MockNewInvoicePayload};

#[test_case(Some("supersecrettoken") => StatusCode::OK; "Correct token")]
#[test_case(Some("I am the wrong token!") => StatusCode::UNAUTHORIZED; "Wrong token")]
#[test_case(None => StatusCode::UNAUTHORIZED; "Missing token")]
#[tokio::test]
async fn bearer_auth(token: Option<&str>) -> StatusCode {
    init_logger();
    let mock_daemon = MockDaemon::new_mock_daemon().await;

    let store = Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap();
    let payment_gateway = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        store,
    )
    .daemon_url(mock_daemon.url(""))
    .build()
    .await
    .unwrap();
    info!("Payment gateway created.");

    // Deliberately not starting the payment gateway itself, because we don't
    // need it for this test.

    let config = load_config(&PathBuf::from(Config::DEFAULT_PATH));
    let server = build_server(&config, payment_gateway).await;
    let address = server.internal_ipv4_address().unwrap();
    tokio::spawn(async {
        run_server(server).await;
    });

    let mut client = GatewayClient::default();

    client.url = Uri::from_str(&format!("https://{}:{}", address.ip(), address.port())).unwrap();
    client.token = token.map(ToString::to_string);

    let new_invoice_response = client
        .new_invoice(MockNewInvoicePayload::default())
        .await
        .expect("failed to call new invoice endpoint");
    debug!("Checkout response: {:?}", new_invoice_response);
    new_invoice_response.status()
}
