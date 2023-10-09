use std::time::Duration;

use acceptxmr::{
    storage::{
        stores::{InMemory, Sled, Sqlite},
        Storage,
    },
    PaymentGatewayBuilder, SubIndex,
};
use test_case::test_case;
use testing_utils::{init_logger, new_temp_dir, MockDaemon, PRIMARY_ADDRESS, PRIVATE_VIEW_KEY};

#[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
#[test_case(InMemory::new(); "in-memory")]
#[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
#[tokio::test]
async fn reproducible_rand<S>(store: S)
where
    S: Storage + 'static,
{
    // Setup.
    init_logger();
    let mock_daemon = MockDaemon::new_mock_daemon().await;

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        store,
    )
    // Faster scan rate so the update is received sooner.
    .scan_interval(Duration::from_millis(100))
    .daemon_url(mock_daemon.url(""))
    .account_index(1)
    .seed(1)
    .build()
    .await
    .expect("failed to build payment gateway");

    // Run it.
    payment_gateway
        .run()
        .await
        .expect("failed to run payment gateway");

    // Add the invoice.
    let invoice_id = payment_gateway
        .new_invoice(1, 5, 10, "test invoice".to_string())
        .await
        .expect("failed to add new invoice to payment gateway for tracking");
    let mut subscriber = payment_gateway
        .subscribe(invoice_id)
        .expect("invoice does not exist");

    // Get initial update.
    let update = subscriber
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");

    // Check that it is as expected.
    assert_eq!(update.index(), SubIndex::new(1, 97));
}
