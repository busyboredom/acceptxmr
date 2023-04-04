use acceptxmr::{storage::stores::Sqlite, PaymentGatewayBuilder};
use acceptxmr_server::{load_config, run_server};
use hyper::StatusCode;
use log::{debug, info};
use test_case::test_case;

use crate::common::{init_logger, GatewayClient, PRIMARY_ADDRESS, PRIVATE_VIEW_KEY};

#[test_case(Some("supersecrettoken") => StatusCode::OK; "Correct token")]
#[test_case(Some("I am the wrong token!") => StatusCode::UNAUTHORIZED; "Wrong token")]
#[test_case(None => StatusCode::UNAUTHORIZED; "Missing token")]
#[tokio::test]
async fn bearer_auth(token: Option<&str>) -> StatusCode {
    init_logger();

    let store = Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap();
    let payment_gateway = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        store,
    )
    .build()
    .unwrap();
    info!("Payment gateway created.");

    // Deliberately not starting the payment gateway itself, because we don't
    // need it for this test.

    let config = load_config();
    tokio::spawn(run_server(config, payment_gateway));

    let mut client = GatewayClient::default();
    client.token = token.map(|s| s.to_string());

    let checkout_response = client
        .checkout()
        .await
        .expect("failed to call `checkout` endpoint");
    debug!("Checkout response: {:?}", checkout_response);
    checkout_response.status()
}
