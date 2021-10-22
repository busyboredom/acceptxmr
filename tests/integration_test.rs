mod common;

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
