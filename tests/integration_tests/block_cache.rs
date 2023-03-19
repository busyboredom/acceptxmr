use std::{
    fmt::{Debug, Display},
    time::Duration,
};

use acceptxmr::{
    storage::{
        stores::{InMemory, Sled, Sqlite},
        InvoiceStorage,
    },
    PaymentGatewayBuilder, SubIndex,
};
use test_case::test_case;
use tokio::runtime::Runtime;

use crate::common::{
    init_logger, new_temp_dir, MockDaemon, MockInvoice, PRIMARY_ADDRESS, PRIVATE_VIEW_KEY,
};

#[test_case(Sled::new(&new_temp_dir(), "tree").unwrap())]
#[test_case(InMemory::new())]
#[test_case(Sqlite::new(":memory:", "invoices").unwrap())]
fn block_cache_skip_ahead<'a, S, E, I>(store: S)
where
    S: InvoiceStorage<Error = E, Iter<'a> = I> + 'static,
    E: Debug + Display + Send,
    I: Iterator,
{
    // Setup.
    init_logger();
    let mock_daemon = MockDaemon::new_mock_daemon();
    let rt = Runtime::new().expect("failed to create tokio runtime");

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        store,
    )
    // Faster scan rate so the update is received sooner.
    .scan_interval(Duration::from_millis(200))
    .daemon_url(mock_daemon.url(""))
    .seed(1)
    .build()
    .expect("failed to build payment gateway");

    // Run it.
    rt.block_on(async {
        payment_gateway
            .run()
            .await
            .expect("failed to run payment gateway");

        assert_eq!(payment_gateway.cache_height(), 2477656);

        mock_daemon.mock_daemon_height(2477666);

        tokio::time::timeout(Duration::from_millis(1000), async {
            while payment_gateway.cache_height() != 2477665 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("timed out waiting for gateway to fast forward");
    })
}

#[test_case(Sled::new(&new_temp_dir(), "tree").unwrap())]
#[test_case(InMemory::new())]
#[test_case(Sqlite::new(":memory:", "invoices").unwrap())]
fn fix_reorg<'a, S, E, I>(store: S)
where
    S: InvoiceStorage<Error = E, Iter<'a> = I> + 'static,
    E: Debug + Display + Send,
    I: Iterator,
{
    // Setup.
    init_logger();
    let mock_daemon = MockDaemon::new_mock_daemon();
    let rt = Runtime::new().expect("failed to create tokio runtime");

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
    .expect("failed to build payment gateway");

    // Run it.
    rt.block_on(async {
        payment_gateway
            .run()
            .await
            .expect("failed to run payment gateway");

        // Add the invoice.
        let invoice_id = payment_gateway
            .new_invoice(70000000, 2, 7, "invoice".to_string())
            .expect("failed to add new invoice to payment gateway for tracking");
        let mut subscriber = payment_gateway
            .subscribe(invoice_id)
            .expect("invoice does not exist");

        // Get initial update.
        let update = subscriber
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        let mut expected = MockInvoice::new(
            Some(update.address().to_string()),
            SubIndex::new(1, 97),
            2477657,
            70000000,
            2,
            7,
            "invoice".to_string(),
        );

        // Check that it is as expected.
        expected.assert_eq(&update);

        mock_daemon.mock_daemon_height(2477658);

        let update = subscriber
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        expected.amount_paid = 37419570;
        expected.expires_in = 6;
        expected.current_height = 2477658;
        expected.assert_eq(&update);

        // Reorg to invalidate payment.
        mock_daemon.mock_alt_2477657();
        mock_daemon.mock_alt_2477658();

        subscriber
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect_err("should not have received an update, but did");

        mock_daemon.mock_daemon_height(24776659);

        let update = subscriber
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        expected.amount_paid = 0;
        expected.expires_in = 5;
        expected.current_height = 2477659;
        expected.assert_eq(&update);
    })
}
