mod common;

use std::time::Duration;

use tokio::runtime::Runtime;

use acceptxmr::PaymentGatewayBuilder;

#[test]
fn run_payment_gateway() {
    // Setup.
    common::init_logger();
    let temp_dir = common::new_temp_dir();
    let mock_daemon = common::new_mock_daemon();
    let rt = Runtime::new().expect("failed to create tokio runtime");

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway =
        PaymentGatewayBuilder::new(common::PRIVATE_VIEW_KEY, common::PUBLIC_SPEND_KEY)
            .db_path(
                temp_dir
                    .path()
                    .to_str()
                    .expect("failed to get temporary directory path"),
            )
            .daemon_url(&mock_daemon.url(""))
            .build();

    // Run it.
    rt.block_on(async {
        payment_gateway
            .run()
            .await
            .expect("failed to run payment gateway");
    })
}

#[test]
fn new_payment() {
    // Setup.
    common::init_logger();
    let temp_dir = common::new_temp_dir();
    let mock_daemon = common::new_mock_daemon();
    let rt = Runtime::new().expect("failed to create tokio runtime");

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway =
        PaymentGatewayBuilder::new(common::PRIVATE_VIEW_KEY, common::PUBLIC_SPEND_KEY)
            .db_path(
                temp_dir
                    .path()
                    .to_str()
                    .expect("failed to get temporary directory path"),
            )
            // Faster scan rate so the update is received sooner.
            .scan_interval(Duration::from_millis(100))
            .daemon_url(&mock_daemon.url(""))
            .build();

    // Run it.
    rt.block_on(async {
        payment_gateway
            .run()
            .await
            .expect("failed to run payment gateway");

        // Add the payment.
        let mut subscriber = payment_gateway
            .new_payment(1, 5, 10)
            .await
            .expect("failed to add new payment to paymend gateway for tracking");

        // Get initial update.
        let update = subscriber
            .recv()
            .expect("failed to retreive payment update");

        // Check that it is as expected.
        assert_eq!(update.amount_requested(), 1);
        assert_eq!(update.amount_paid(), 0);
        assert!(!update.is_expired());
        assert_eq!(update.expiration_at() - update.started_at(), 10);
        assert_eq!(update.started_at(), update.current_height());
        assert_eq!(update.confirmations_required(), 5);
        assert_eq!(update.confirmations(), None);
    })
}
