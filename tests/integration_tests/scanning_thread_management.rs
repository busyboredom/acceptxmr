use tokio::runtime::Runtime;

use acceptxmr::{AcceptXmrError, PaymentGatewayBuilder, PaymentGatewayStatus};

use crate::common::{self, MockDaemon};

#[test]
fn run_payment_gateway() {
    // Setup.
    common::init_logger();
    let temp_dir = common::new_temp_dir();
    let mock_daemon = MockDaemon::new_mock_daemon();
    let rt = Runtime::new().expect("failed to create tokio runtime");

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway = PaymentGatewayBuilder::new(
        common::PRIVATE_VIEW_KEY.to_string(),
        common::PRIMARY_ADDRESS.to_string(),
    )
    .db_path(
        temp_dir
            .path()
            .to_str()
            .expect("failed to get temporary directory path")
            .to_string(),
    )
    .daemon_url(&mock_daemon.url(""))
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
    common::init_logger();
    let temp_dir = common::new_temp_dir();
    let mock_daemon = MockDaemon::new_mock_daemon();
    let rt = Runtime::new().expect("failed to create tokio runtime");

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway = PaymentGatewayBuilder::new(
        common::PRIVATE_VIEW_KEY.to_string(),
        common::PRIMARY_ADDRESS.to_string(),
    )
    .db_path(
        temp_dir
            .path()
            .to_str()
            .expect("failed to get temporary directory path")
            .to_string(),
    )
    .daemon_url(&mock_daemon.url(""))
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
    common::init_logger();
    let temp_dir = common::new_temp_dir();
    let mock_daemon = MockDaemon::new_mock_daemon();
    let rt = Runtime::new().expect("failed to create tokio runtime");

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway = PaymentGatewayBuilder::new(
        common::PRIVATE_VIEW_KEY.to_string(),
        common::PRIMARY_ADDRESS.to_string(),
    )
    .db_path(
        temp_dir
            .path()
            .to_str()
            .expect("failed to get temporary directory path")
            .to_string(),
    )
    .daemon_url(&mock_daemon.url(""))
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
