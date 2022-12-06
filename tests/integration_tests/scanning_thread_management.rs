use tokio::runtime::Runtime;

use acceptxmr::{
    storage::stores::Sled, AcceptXmrError, PaymentGatewayBuilder, PaymentGatewayStatus,
};

use crate::common::{init_logger, new_temp_dir, MockDaemon, PRIMARY_ADDRESS, PRIVATE_VIEW_KEY};

#[test]
fn run_payment_gateway() {
    // Setup.
    init_logger();
    let temp_dir = new_temp_dir();
    let mock_daemon = MockDaemon::new_mock_daemon();
    let rt = Runtime::new().expect("failed to create tokio runtime");

    let store = Sled::new(
        temp_dir
            .path()
            .to_str()
            .expect("failed to get temporary directory path"),
        "tree name",
    )
    .expect("failed to create sled storage layer.");

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        store,
    )
    .daemon_url(mock_daemon.url(""))
    .build()
    .expect("failed to build payment gateway");

    // Run it.
    rt.block_on(async {
        payment_gateway
            .run()
            .await
            .expect("failed to run payment gateway");
    })
}

#[test]
fn cannot_run_payment_gateway_twice() {
    // Setup.
    init_logger();
    let temp_dir = new_temp_dir();
    let mock_daemon = MockDaemon::new_mock_daemon();
    let rt = Runtime::new().expect("failed to create tokio runtime");

    let store = Sled::new(
        temp_dir
            .path()
            .to_str()
            .expect("failed to get temporary directory path"),
        "tree name",
    )
    .expect("failed to create sled storage layer.");

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        store,
    )
    .daemon_url(mock_daemon.url(""))
    .build()
    .expect("failed to build payment gateway");

    // Run it.
    rt.block_on(async {
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
    })
}

#[test]
fn stop_payment_gateway() {
    // Setup.
    init_logger();
    let temp_dir = new_temp_dir();
    let mock_daemon = MockDaemon::new_mock_daemon();
    let rt = Runtime::new().expect("failed to create tokio runtime");

    let store = Sled::new(
        temp_dir
            .path()
            .to_str()
            .expect("failed to get temporary directory path"),
        "tree name",
    )
    .expect("failed to create sled storage layer.");

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        store,
    )
    .daemon_url(mock_daemon.url(""))
    .build()
    .expect("failed to build payment gateway");

    // Run it.
    rt.block_on(async {
        payment_gateway
            .run()
            .await
            .expect("failed to run payment gateway");

        assert!(matches!(
            payment_gateway.status(),
            PaymentGatewayStatus::Running,
        ));

        assert!(payment_gateway.stop().is_ok());
    })
}
