use std::{
    fmt::{Debug, Display},
    time::Duration,
};

use test_case::test_case;
use tokio::runtime::Runtime;

use acceptxmr::{
    storage::{
        stores::{InMemory, Sled},
        InvoiceStorage,
    },
    PaymentGatewayBuilder, SubIndex,
};

use crate::common::{init_logger, new_temp_dir, MockDaemon, PRIMARY_ADDRESS, PRIVATE_VIEW_KEY};

#[test_case(Sled::new(new_temp_dir().path().to_str().unwrap(), "tree").unwrap())]
#[test_case(InMemory::new())]
fn new_invoice<'a, S, E, I>(store: S)
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
            .new_invoice(1, 5, 10, "test invoice".to_string())
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

        // Check that it is as expected.
        assert_eq!(update.amount_requested(), 1);
        assert_eq!(update.amount_paid(), 0);
        assert!(!update.is_expired());
        assert!(!update.is_confirmed());
        assert_eq!(update.expiration_height() - update.creation_height(), 10);
        assert_eq!(update.creation_height(), update.current_height());
        assert_eq!(update.confirmations_required(), 5);
        assert_eq!(update.confirmations(), None);
        assert_eq!(update.description(), "test invoice".to_string());
        assert_eq!(
            update.current_height(),
            payment_gateway
                .daemon_height()
                .await
                .expect("failed to retrieve daemon height")
        );
    })
}

#[test_case(Sled::new(new_temp_dir().path().to_str().unwrap(), "tree").unwrap())]
#[test_case(InMemory::new())]
fn track_parallel_invoices<'a, S, E, I>(store: S)
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
            .new_invoice(70000000, 2, 7, "invoice 1".to_string())
            .expect("failed to add new invoice to payment gateway for tracking");
        let mut subscriber_1 = payment_gateway
            .subscribe(invoice_id)
            .expect("invoice does not exist");

        // Get initial update.
        let update = subscriber_1
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        // Check that it is as expected.
        assert_eq!(update.amount_requested(), 70000000);
        assert_eq!(update.index(), SubIndex::new(1, 97));
        assert_eq!(update.amount_paid(), 0);
        assert!(!update.is_expired());
        assert!(!update.is_confirmed());
        assert_eq!(update.expiration_height() - update.creation_height(), 7);
        assert_eq!(update.creation_height(), update.current_height());
        assert_eq!(update.confirmations_required(), 2);
        assert_eq!(update.confirmations(), None);

        // Add the invoice.
        let invoice_id = payment_gateway
            .new_invoice(70000000, 2, 7, "invoice 2".to_string())
            .expect("failed to add new invoice to payment gateway for tracking");
        let mut subscriber_2 = payment_gateway
            .subscribe(invoice_id)
            .expect("invoice does not exist");

        // Get initial update.
        let update = subscriber_2
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        // Check that it is as expected.
        assert_eq!(update.amount_requested(), 70000000);
        assert_eq!(update.index(), SubIndex::new(1, 138));
        assert_eq!(update.amount_paid(), 0);
        assert!(!update.is_expired());
        assert!(!update.is_confirmed());
        assert_eq!(update.expiration_height() - update.creation_height(), 7);
        assert_eq!(update.creation_height(), update.current_height());
        assert_eq!(update.confirmations_required(), 2);
        assert_eq!(update.confirmations(), None);

        // Add double transfer to txpool.
        let txpool_hashes_mock =
            mock_daemon.mock_txpool_hashes("tests/rpc_resources/txpool_hashes_with_payment.json");
        // Mock for these transactions themselves is unnecessary, because they are all in block
        // 2477657.

        // Get update.
        let update = subscriber_1
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        // Check that it is as expected.
        assert_eq!(update.amount_requested(), 70000000);
        assert_eq!(update.index(), SubIndex::new(1, 97));
        assert_eq!(update.amount_paid(), 37419570);
        assert!(!update.is_expired());
        assert!(!update.is_confirmed());
        assert_eq!(update.expiration_height() - update.creation_height(), 7);
        assert_eq!(update.creation_height(), update.current_height());
        assert_eq!(update.confirmations_required(), 2);
        assert_eq!(update.confirmations(), None);

        // Get update.
        let update = subscriber_2
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        // Check that it is as expected.
        assert_eq!(update.amount_requested(), 70000000);
        assert_eq!(update.index(), SubIndex::new(1, 138));
        assert_eq!(update.amount_paid(), 37419570);
        assert!(!update.is_expired());
        assert!(!update.is_confirmed());
        assert_eq!(update.expiration_height() - update.creation_height(), 7);
        assert_eq!(update.creation_height(), update.current_height());
        assert_eq!(update.confirmations_required(), 2);
        assert_eq!(update.confirmations(), None);

        // Check that the mock server did in fact receive the requests.
        assert!(txpool_hashes_mock.hits() > 0);

        // Move forward a few blocks.
        mock_daemon.mock_txpool_hashes("tests/rpc_resources/txpool_hashes.json");
        for height in 2477658..2477663 {
            let height_mock = mock_daemon.mock_daemon_height(height);

            let update = subscriber_1
                .recv_timeout(Duration::from_millis(5000))
                .await
                .expect("timeout waiting for invoice update")
                .expect("subscription channel is closed");

            assert_eq!(update.amount_requested(), 70000000);
            assert_eq!(update.index(), SubIndex::new(1, 97));
            assert_eq!(update.amount_paid(), 37419570);
            assert!(!update.is_expired());
            assert!(!update.is_confirmed());
            assert_eq!(update.expiration_height() - update.creation_height(), 7);
            assert_eq!(update.current_height(), height);
            assert_eq!(update.confirmations_required(), 2);
            assert_eq!(update.confirmations(), None);

            let update = subscriber_2
                .recv_timeout(Duration::from_millis(5000))
                .await
                .expect("timeout waiting for invoice update")
                .expect("subscription channel is closed");

            assert_eq!(update.amount_requested(), 70000000);
            assert_eq!(update.index(), SubIndex::new(1, 138));
            assert_eq!(update.amount_paid(), 37419570);
            assert!(!update.is_expired());
            assert!(!update.is_confirmed());
            assert_eq!(update.expiration_height() - update.creation_height(), 7);
            assert_eq!(update.current_height(), height);
            assert_eq!(update.confirmations_required(), 2);
            assert_eq!(update.confirmations(), None);

            assert!(height_mock.hits() > 0);
        }

        // Put second payment in txpool.
        let txpool_hashes_mock =
            mock_daemon.mock_txpool_hashes("tests/rpc_resources/txpool_hashes_with_payment_2.json");
        let txpool_transactions_mock = mock_daemon.mock_txpool_transactions(
            "tests/rpc_resources/transaction_hashes_with_payment_2.json",
            "tests/rpc_resources/transactions_with_payment_2.json",
        );

        // Invoice 1 should be paid now.
        let update = subscriber_1
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        assert_eq!(update.amount_requested(), 70000000);
        assert_eq!(update.index(), SubIndex::new(1, 97));
        assert_eq!(update.amount_paid(), 74839140);
        assert!(!update.is_expired());
        assert!(!update.is_confirmed());
        assert_eq!(update.expiration_height() - update.creation_height(), 7);
        assert_eq!(update.current_height(), 2477662);
        assert_eq!(update.confirmations_required(), 2);
        assert_eq!(update.confirmations(), Some(0));

        // Invoice 2 should not have an update.
        subscriber_2
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect_err("should not have received an update, but did");

        assert!(txpool_hashes_mock.hits() > 0);
        assert!(txpool_transactions_mock.hits() > 0);

        // Move forward a block
        // (getting update after txpool change, so there's no data race between the scanner and
        // these two mock changes).
        let txpool_hashes_mock =
            mock_daemon.mock_txpool_hashes("tests/rpc_resources/txpool_hashes.json");
        subscriber_1
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");
        subscriber_2
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect_err("should not have received an update, but did");
        let height_mock = mock_daemon.mock_daemon_height(2477663);

        let update = subscriber_1
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        assert_eq!(update.amount_requested(), 70000000);
        assert_eq!(update.index(), SubIndex::new(1, 97));
        assert_eq!(update.amount_paid(), 74839140);
        assert!(!update.is_expired());
        assert!(!update.is_confirmed());
        assert_eq!(update.expiration_height() - update.creation_height(), 7);
        assert_eq!(update.current_height(), 2477663);
        assert_eq!(update.confirmations_required(), 2);
        assert_eq!(update.confirmations(), Some(1));

        let update = subscriber_2
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        assert_eq!(update.amount_requested(), 70000000);
        assert_eq!(update.index(), SubIndex::new(1, 138));
        assert_eq!(update.amount_paid(), 37419570);
        assert!(!update.is_expired());
        assert!(!update.is_confirmed());
        assert_eq!(update.expiration_height() - update.creation_height(), 7);
        assert_eq!(update.current_height(), 2477663);
        assert_eq!(update.confirmations_required(), 2);
        assert_eq!(update.confirmations(), None);

        assert!(txpool_hashes_mock.hits() > 0);
        assert!(height_mock.hits() > 0);

        // Move forward a block.
        let height_mock = mock_daemon.mock_daemon_height(2477664);

        let update = subscriber_1
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        assert_eq!(update.amount_requested(), 70000000);
        assert_eq!(update.index(), SubIndex::new(1, 97));
        assert_eq!(update.amount_paid(), 74839140);
        assert!(!update.is_expired());
        assert!(update.is_confirmed());
        assert_eq!(update.expiration_height() - update.creation_height(), 7);
        assert_eq!(update.current_height(), 2477664);
        assert_eq!(update.confirmations_required(), 2);
        assert_eq!(update.confirmations(), Some(2));

        let update = subscriber_2
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        assert_eq!(update.amount_requested(), 70000000);
        assert_eq!(update.index(), SubIndex::new(1, 138));
        assert_eq!(update.amount_paid(), 37419570);
        assert!(update.is_expired());
        assert!(!update.is_confirmed());
        assert_eq!(update.expiration_height() - update.creation_height(), 7);
        assert_eq!(update.current_height(), 2477664);
        assert_eq!(update.confirmations_required(), 2);
        assert_eq!(update.confirmations(), None);

        assert!(txpool_hashes_mock.hits() > 0);
        assert!(height_mock.hits() > 0);
    })
}

#[test_case(Sled::new(new_temp_dir().path().to_str().unwrap(), "tree").unwrap())]
#[test_case(InMemory::new())]
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

        tokio::time::sleep(tokio::time::Duration::from_millis(600)).await;
        assert_eq!(payment_gateway.cache_height(), 2477665);
    })
}

#[test_case(Sled::new(new_temp_dir().path().to_str().unwrap(), "tree").unwrap())]
#[test_case(InMemory::new())]
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

        // Check that it is as expected.
        assert_eq!(update.amount_requested(), 70000000);
        assert_eq!(update.index(), SubIndex::new(1, 97));
        assert_eq!(update.amount_paid(), 0);
        assert!(!update.is_expired());
        assert!(!update.is_confirmed());
        assert_eq!(update.expiration_height() - update.creation_height(), 7);
        assert_eq!(update.creation_height(), update.current_height());
        assert_eq!(update.confirmations_required(), 2);
        assert_eq!(update.confirmations(), None);

        mock_daemon.mock_daemon_height(2477658);

        let update = subscriber
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        assert_eq!(update.amount_requested(), 70000000);
        assert_eq!(update.index(), SubIndex::new(1, 97));
        assert_eq!(update.amount_paid(), 37419570);
        assert!(!update.is_expired());
        assert!(!update.is_confirmed());
        assert_eq!(update.expiration_height() - update.creation_height(), 7);
        assert_eq!(update.current_height(), 2477658);
        assert_eq!(update.confirmations_required(), 2);
        assert_eq!(update.confirmations(), None);

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

        assert_eq!(update.amount_requested(), 70000000);
        assert_eq!(update.index(), SubIndex::new(1, 97));
        assert_eq!(update.amount_paid(), 0);
        assert!(!update.is_expired());
        assert!(!update.is_confirmed());
        assert_eq!(update.expiration_height() - update.creation_height(), 7);
        assert_eq!(update.current_height(), 2477659);
        assert_eq!(update.confirmations_required(), 2);
        assert_eq!(update.confirmations(), None);
    })
}

#[test_case(Sled::new(new_temp_dir().path().to_str().unwrap(), "tree").unwrap())]
#[test_case(InMemory::new())]
fn reproducible_rand<'a, S, E, I>(store: S)
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
            .new_invoice(1, 5, 10, "test invoice".to_string())
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

        // Check that it is as expected.
        assert_eq!(update.index(), SubIndex::new(1, 97));
    })
}
