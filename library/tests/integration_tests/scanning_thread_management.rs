use acceptxmr::{
    storage::stores::Sled, AcceptXmrError, PaymentGatewayBuilder, PaymentGatewayStatus,
};
use testing_utils::{init_logger, new_temp_dir, MockDaemon, PRIMARY_ADDRESS, PRIVATE_VIEW_KEY};

#[tokio::test]
async fn run_payment_gateway() {
    // Setup.
    init_logger();
    let temp_dir = new_temp_dir();
    let mock_daemon = MockDaemon::new_mock_daemon().await;

    let store = Sled::new(&temp_dir, "invoices", "output keys", "height")
        .expect("failed to create sled storage layer.");

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        store,
    )
    .daemon_url(mock_daemon.url(""))
    .build()
    .await
    .expect("failed to build payment gateway");

    // Run it.
    payment_gateway
        .run()
        .await
        .expect("failed to run payment gateway");
}

#[tokio::test]
async fn cannot_run_payment_gateway_twice() {
    // Setup.
    init_logger();
    let temp_dir = new_temp_dir();
    let mock_daemon = MockDaemon::new_mock_daemon().await;

    let store = Sled::new(&temp_dir, "invoices", "output keys", "height")
        .expect("failed to create sled storage layer.");

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        store,
    )
    .daemon_url(mock_daemon.url(""))
    .build()
    .await
    .expect("failed to build payment gateway");

    // Run it.
    payment_gateway
        .run()
        .await
        .expect("failed to run payment gateway");

    assert!(
        matches!(
            payment_gateway.run().await,
            Err(AcceptXmrError::AlreadyRunning)
        ),
        "payment gateway was run twice"
    );
}

#[tokio::test]
async fn stop_payment_gateway() {
    // Setup.
    init_logger();
    let temp_dir = new_temp_dir();
    let mock_daemon = MockDaemon::new_mock_daemon().await;

    let store = Sled::new(&temp_dir, "invoices", "output keys", "height")
        .expect("failed to create sled storage layer.");

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        store,
    )
    .daemon_url(mock_daemon.url(""))
    .build()
    .await
    .expect("failed to build payment gateway");

    // Run it.
    payment_gateway
        .run()
        .await
        .expect("failed to run payment gateway");

    assert!(matches!(
        payment_gateway.status().await,
        PaymentGatewayStatus::Running,
    ));

    assert!(payment_gateway.stop().await.is_ok());
}
