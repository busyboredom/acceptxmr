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

#[test_case(Sled::new(&new_temp_dir(), "tree").unwrap())]
#[test_case(InMemory::new())]
#[test_case(Sqlite::new(":memory:", "invoices").unwrap())]
fn default_account_index<'a, S, E, I>(store: S)
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

        let mut expected = MockInvoice::new(
            Some(update.address().to_string()),
            SubIndex::new(0, 97),
            2477657,
            1,
            5,
            10,
            "test invoice".to_string(),
        );

        // Check that it is as expected.
        expected.assert_eq(&update);
        assert_eq!(
            update.current_height(),
            payment_gateway
                .daemon_height()
                .await
                .expect("failed to retrieve daemon height")
        );

        // Add transfer to txpool.
        let _txpool_hashes_mock = mock_daemon
            .mock_txpool_hashes("tests/rpc_resources/txpools/hashes_with_payment_account_0.json");
        let _transactions_mock = mock_daemon.mock_transactions(
            "tests/rpc_resources/transactions/hashes_with_payment_account_0.json",
            "tests/rpc_resources/transactions/txs_with_payment_account_0.json",
        );

        // Get update.
        let update = subscriber
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        expected.amount_paid = 1468383460;
        expected.confirmations = Some(0);
        expected.assert_eq(&update);
    })
}

#[test_case(Sled::new(&new_temp_dir(), "tree").unwrap())]
#[test_case(InMemory::new())]
#[test_case(Sqlite::new(":memory:", "invoices").unwrap())]
fn zero_conf_invoice<'a, S, E, I>(store: S)
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
            .new_invoice(37419570, 0, 10, "test invoice".to_string())
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
            37419570,
            0,
            10,
            "test invoice".to_string(),
        );

        // Check that it is as expected.
        expected.assert_eq(&update);

        // Add transfer to txpool.
        let _txpool_hashes_mock =
            mock_daemon.mock_txpool_hashes("tests/rpc_resources/txpools/hashes_with_payment.json");

        // Get update.
        let update = subscriber
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        expected.amount_paid = 37419570;
        expected.confirmations = Some(0);
        expected.is_confirmed = true;
        expected.assert_eq(&update);
    })
}

#[test_case(Sled::new(&new_temp_dir(), "tree").unwrap())]
#[test_case(InMemory::new())]
#[test_case(Sqlite::new(":memory:", "invoices").unwrap())]
fn timelock_rejection<'a, S, E, I>(store: S)
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
            .new_invoice(123, 1, 1, "test invoice".to_string())
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

        let expected = MockInvoice::new(
            Some(update.address().to_string()),
            SubIndex::new(0, 97),
            2477657,
            123,
            1,
            1,
            "test invoice".to_string(),
        );

        // Check that it is as expected.
        expected.assert_eq(&update);

        // Add transfer to txpool.
        let _txpool_hashes_mock = mock_daemon
            .mock_txpool_hashes("tests/rpc_resources/txpools/hashes_with_payment_timelock.json");
        let _transactions_mock = mock_daemon.mock_transactions(
            "tests/rpc_resources/transactions/hashes_with_payment_timelock.json",
            "tests/rpc_resources/transactions/txs_with_payment_timelock.json",
        );

        // There shouldn't be any update.
        subscriber
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect_err("timeout waiting for invoice update");
    })
}

#[test_case(Sled::new(&new_temp_dir(), "tree").unwrap())]
#[test_case(InMemory::new())]
#[test_case(Sqlite::new(":memory:", "invoices").unwrap())]
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

        let mut expected_1 = MockInvoice::new(
            Some(update.address().to_string()),
            SubIndex::new(1, 97),
            2477657,
            70000000,
            2,
            7,
            "invoice 1".to_string(),
        );

        // Check that it is as expected.
        expected_1.assert_eq(&update);

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

        let mut expected_2 = expected_1.clone();
        expected_2.address = Some(update.address().to_string());
        expected_2.index = SubIndex::new(1, 138);
        expected_2.description = "invoice 2".to_string();

        // Check that it is as expected.
        expected_2.assert_eq(&update);

        // Add double transfer to txpool.
        let txpool_hashes_mock =
            mock_daemon.mock_txpool_hashes("tests/rpc_resources/txpools/hashes_with_payment.json");
        // Mock for these transactions themselves is unnecessary, because they are all
        // in block 2477657.

        // Get update.
        let update = subscriber_1
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        // Check that it is as expected.
        expected_1.amount_paid = 37419570;
        expected_1.assert_eq(&update);

        // Get update.
        let update = subscriber_2
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        // Check that it is as expected.
        expected_2.amount_paid = 37419570;
        expected_2.assert_eq(&update);

        // Check that the mock server did in fact receive the requests.
        assert!(txpool_hashes_mock.hits() > 0);

        // Mock txpool with no payments (as if the payment moved to a block).
        mock_daemon.mock_txpool_hashes("tests/rpc_resources/txpools/hashes.json");

        // Both invoices should now show zero paid.
        let update = subscriber_1
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");
        assert_eq!(update.amount_paid(), 0);
        let update = subscriber_2
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");
        assert_eq!(update.amount_paid(), 0);

        // Move forward a few blocks.
        for height in 2477658..2477663 {
            let height_mock = mock_daemon.mock_daemon_height(height);

            let update = subscriber_1
                .recv_timeout(Duration::from_millis(5000))
                .await
                .expect("timeout waiting for invoice update")
                .expect("subscription channel is closed");

            expected_1.expires_in = 2477664 - height;
            expected_1.current_height = height;
            expected_1.assert_eq(&update);

            let update = subscriber_2
                .recv_timeout(Duration::from_millis(5000))
                .await
                .expect("timeout waiting for invoice update")
                .expect("subscription channel is closed");

            expected_2.expires_in = 2477664 - height;
            expected_2.current_height = height;
            expected_2.assert_eq(&update);

            assert!(height_mock.hits() > 0);
        }

        // Put second payment in txpool.
        let txpool_hashes_mock = mock_daemon
            .mock_txpool_hashes("tests/rpc_resources/txpools/hashes_with_payment_2.json");
        let txpool_transactions_mock = mock_daemon.mock_txpool_transactions(
            "tests/rpc_resources/transactions/hashes_with_payment_2.json",
            "tests/rpc_resources/transactions/txs_with_payment_2.json",
        );

        // Invoice 1 should be paid now.
        let update = subscriber_1
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        expected_1.amount_paid = 74839140;
        expected_1.confirmations = Some(0);
        expected_1.assert_eq(&update);

        // Invoice 2 should not have an update.
        subscriber_2
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect_err("should not have received an update, but did");

        assert!(txpool_hashes_mock.hits() > 0);
        assert!(txpool_transactions_mock.hits() > 0);

        // Move forward a block
        // (getting update after txpool change, so there's no data race between the
        // scanner and these two mock changes).
        let txpool_hashes_mock =
            mock_daemon.mock_txpool_hashes("tests/rpc_resources/txpools/hashes.json");
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

        expected_1.confirmations = Some(1);
        expected_1.expires_in = 1;
        expected_1.current_height = 2477663;
        expected_1.assert_eq(&update);

        let update = subscriber_2
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        expected_2.expires_in = 1;
        expected_2.current_height = 2477663;
        expected_2.assert_eq(&update);

        assert!(txpool_hashes_mock.hits() > 0);
        assert!(height_mock.hits() > 0);

        // Move forward a block.
        let height_mock = mock_daemon.mock_daemon_height(2477664);

        let update = subscriber_1
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        expected_1.confirmations = Some(2);
        expected_1.is_confirmed = true;
        expected_1.expires_in = 0;
        expected_1.current_height = 2477664;
        expected_1.assert_eq(&update);

        let update = subscriber_2
            .recv_timeout(Duration::from_millis(5000))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        expected_2.expires_in = 0;
        expected_2.is_expired = true;
        expected_2.current_height = 2477664;
        expected_2.assert_eq(&update);

        assert!(txpool_hashes_mock.hits() > 0);
        assert!(height_mock.hits() > 0);
    })
}
