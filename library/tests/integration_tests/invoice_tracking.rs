use std::time::Duration;

use acceptxmr::{
    storage::{
        stores::{InMemory, Sled, Sqlite},
        OutputId, OutputKeyStorage, OutputPubKey, Storage,
    },
    PaymentGatewayBuilder, SubIndex,
};
use monero::consensus::deserialize;
use test_case::test_case;

use crate::common::{
    init_logger, new_temp_dir, MockDaemon, MockInvoice, PRIMARY_ADDRESS, PRIVATE_VIEW_KEY,
};

#[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
#[test_case(InMemory::new(); "in-memory")]
#[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
#[tokio::test]
async fn new_invoice<S>(store: S)
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
    .build()
    .expect("failed to build payment gateway");

    // Run it.
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
        .recv_timeout(Duration::from_secs(120))
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
}

#[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
#[test_case(InMemory::new(); "in-memory")]
#[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
#[tokio::test]
async fn default_account_index<S>(store: S)
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
    .seed(1)
    .build()
    .expect("failed to build payment gateway");

    // Run it.
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
        .recv_timeout(Duration::from_secs(120))
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
    let _transactions_mock = mock_daemon.mock_transactions(
        "tests/rpc_resources/transactions/hashes_with_payment_account_0.json",
        "tests/rpc_resources/transactions/txs_with_payment_account_0.json",
    );
    let _txpool_hashes_mock = mock_daemon
        .mock_txpool_hashes("tests/rpc_resources/txpools/hashes_with_payment_account_0.json");

    // Get update.
    let update = subscriber
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");

    expected.amount_paid = 1468383460;
    expected.confirmations = Some(0);
    expected.assert_eq(&update);
}

#[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
#[test_case(InMemory::new(); "in-memory")]
#[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
#[tokio::test]
async fn zero_conf_invoice<S>(store: S)
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
    .expect("failed to build payment gateway");

    // Run it.
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
        .recv_timeout(Duration::from_secs(120))
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
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");

    expected.amount_paid = 37419570;
    expected.confirmations = Some(0);
    expected.is_confirmed = true;
    expected.assert_eq(&update);
}

#[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
#[test_case(InMemory::new(); "in-memory")]
#[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
#[tokio::test]
async fn timelock_rejection<S>(store: S)
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
    .seed(1)
    .build()
    .expect("failed to build payment gateway");

    // Run it.
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
        .recv_timeout(Duration::from_secs(120))
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
        .recv_timeout(Duration::from_secs(1))
        .await
        .expect_err("timeout waiting for invoice update");
}

#[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
#[test_case(InMemory::new(); "in-memory")]
#[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
#[tokio::test]
async fn burning_bug<S>(mut store: S)
where
    S: Storage + 'static,
{
    // Setup.
    init_logger();
    let mock_daemon = MockDaemon::new_mock_daemon().await;

    let tx_hex = hex::decode("02000202000bde84861298e38c01c99338d5b005cab9038d26d8e301d442e304a419f406af616b0e66f90a5ff643ee8ff496a25e2caf3d8fccf65895073212a795ae7a7c02000ba09ef20f9da08e02dcb4a301eba60def810cc3b20e9e8801bf9101cdcf04f6ae02f7a50118130528cdc0df212e936b5c95b35ebcdecd443000e94b637782f47a2a47171c0300026f5955625996a0051ff3a419b5fe967fd4a691ad99b2e7dce6fcb35d21821f0d0002b024664bccba2226849a95dfe653c64e02c5498ea88265457c44f85778f018f700027fd35731e1bd4d1ef4bb1f0f008c4b0ba54bb214bb3304fab61c2b5e3a8570f4830101349587e7557632f77ab342e1fb69dc1d106900bc3557e45b9f18d117be363bbc0403909dd7c66c8dba74062fa6d53bcbaa9c3c3a2481dd23d27be3b178449a33303aa17cbf28ae53a1ed402ba4bda9466b92471ce9ddafd5c2ae6e07470ed4d210728a20e76551ee2b320b3fac21c6d823bd6bf2c645e039e226290bc427cc3621930590d99107d6f209fb953c4370924e52a5b1b3df48fbf8306474b69d0998c657aa6aeb1f2f2ee09d5c8e8acf0a32a76aa7f949305829b5a35d53a8b7f72a9960e05ce91198e32bb1f5b049ae2f09aba0975fca8df6f176c947f70170e1514c5c6a75bfcea627e9fcedc7d75fc4a8440b5c331ff2b8a06b0d9c01aca9a1016700a9b4ca3f1f52fccda5354b21bd22bd0e0e895a12925137873dc2a7591e2650ff864e696f540e3c081c4d8d839388447bbba489a0662ca001ad5ac4008d4586d19e4b629957a95b566ad0ce9f78e3e4ac8bd98781eec4d785587f1a3410251141bd0263b04ee2e4948fbd04e63469d6c78b07d16810ad36399e8d751e5437863acd407f54a3814e7f1293a292830cc56882793cb5710bd46a80366c86880ed5c018b026277beb0892896941fe57e5e74a2222627d1dc5d9bd7e89048e0e0308b76103351b7bb3f1a188136a594c1eaa210e35b8c8e6f496cf75cb4df26d41a2a4008fa53018c49b5814d191eb59f79def3599cedae951e1fabe03040aab2a88dda1ebe4280667f758bef886709de3b0cce728b7eec6fb715d167d209532320603b31dc492a21d3804dfe17737c4f50fdbc3cfeb092cdc607bf24fc2d069ca033e1bb17baac9b23db8b853296c82e5daf9d3b1e79bc300b5ba6f9a5f97a13b1d2f27d086c4e237404d2b06df5d2fd16a7f0206e4b534e0b1c2408e9946f0d22a805eb23ea77bf4c5470d7daf0b0ec6df7f923c53197b0cfe68232ae28f3d79472d3a691a5f1c54c3bd31b75510d4ecc54ffafd6c07d4a6ad99a177d544bbef9b08ccf52f7df026fc714f1b7f2501f220d90972c89c10f87dd63fa28f61c1e900cae45f3baa4a6d596f95cb1b603481345360fd728c7f65202b9701caca1d099fe0006eba48e052d52e8b30f116c1107aa98bd8e32288f3a9bd658bbae512d83eec72ecbed4c22b0bbdb34205edb3207c218254a0bc860cf7f9f6a5b67ec934b1517a282d97076f4d43a999aeddca80d6f949eb36b65252b29f9d0053829565d29022fc82c9128ce08733afae38aee26395eaaee03313d75d9f9d6dc1a2a554ca13018d6b8ffd47d097e8f77798d8f5431a93154c5c4588783318115cf9d23f53e296319792368fd762e7aed6c16f404e9cf17345655213f7d95834218ece675a07bf2f07920bfe0bc61ac73ed8fed0d177ab9e116ef0afff097f301216bb9ee80c18a207918b57b6a5dfc59b6a1e9e4420fe9b8355721e384d1af64fa73953b005aeff256d47e8c4666b0995315c039745dbb866fa3c2138d103d53c7a2411c80b8a07eacd709743eda1f7358265358944b017bbeecb8b2e4afd062da7bdcee50d97cfc304ffdb3fb2bf10eb6c79797298afcf7aa9a698f14310896ac1ea3e61044b73a0f9bd4ede0f1907734789b66f0752f2e40cb0aafae3c7ecf284a239e5086205377c5458c1e31762ad6eab90a234ba7699595bf805886b460f32dcae6601d3a8fb42c28d751e7e13ad8a896448688308490e657b24b15c7ade2a6f60aa046148e609e420c8418f945db4df76f4011987870fca43ad2e4dc31c9667cabd0cf192a860df67bec0d794f85d93c99e645a6b27dc51cfe4593a27570d7489ad090e55dde38cd6acaedd04eb88982904d217929e10bb29e599fd3e80a3d5a86202fb629a13cbb47c1b8edd56d1f39b74b1fc6f3e8294e02bcd74e8490344e5c70f001260de40fe26fd7289b5a271460168fdb828dda2d548ca395fc0481188c904bb1c900be39468da0dbd46203addd690bfd53b25b9e3a551a65fc99132abbc0b1aa7de6e2878f6a326aaa73ec10ec17cfb7b16ae83279fa644b5ad69fa4a44093a86c19d31279efc8361f0d71237051689e5538e3409056edb5cdf209c648c9086fa753ea32a85a9254651ed68fb9c4fa89340fdf2e97e84ace81e8aba8be50adea34b61bd4e16e53a49c8a6731e9ea6ac0806aa30cbcf02b54aaab7a604710e5d40354cd2d7ef23863bbc764f0398a1da11597d9a49130d2b22e95bfb62770dbb5815145c5fe442d21b671c2852a8695b58abaedc9de67708d4a18b03fd0f053b77aaecdd12603947be45e4e44a960365ad881f1f79fb098022f4766628a20668de0253abf0ef530ab111368bd8b8a4940dd5fea0c66c37c887e1343d58770de2632ac9ba7400e79c40ff2c4558ef768d8fb1f21d2341a00ecc57ff34aa170b9f665fec6f8c120acb300955e25fd5e04c22edd9b6f939ae2ab8a5bac1210d0ba66f5b2eb7113dc36d469893ca52f7d6110cf1d849b5f32ad308ca1b3935700b76ba4726fff29019f0b4a85a5c0c1e69310f597a37b3580af823bbd6c7dec80ab0aabfe62aa035ec5596f954d6477d47e76090a40d8354694965cb66ab3d280b0465558641f266cd84010b8554a6e394a9a2764ff9e7d355f0e76c5af11e170c847278309f7905d98213e59cc2435e06b8a563b5929676dca3de7eaa0fa970a09a65a845966d8300fa6b941a0fcef6a2daf8a92c1bc69de7a3b6290086ccac9e98d42bec94d2ad302cee609a1bd2e533675a9b5702f30a985a0578b2d823029d").unwrap();
    let tx: monero::Transaction = deserialize(&tx_hex).unwrap();
    let output = &tx.prefix().outputs[1];
    let output_key = OutputPubKey(output.get_pubkeys().unwrap()[0].to_bytes());
    let output_id = OutputId {
        tx_hash: [0; 32], // An intentionally different tx hash.
        index: 1,
    };
    // Insert the key with a different ID so it looks like a re-used key.
    OutputKeyStorage::insert(&mut store, output_key, output_id).unwrap();

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
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");

    let expected = MockInvoice::new(
        Some(update.address().to_string()),
        SubIndex::new(1, 97),
        2477657,
        123,
        1,
        1,
        "test invoice".to_string(),
    );

    // Check that it is as expected.
    expected.assert_eq(&update);

    // Add transfer to txpool.
    let _txpool_hashes_mock =
        mock_daemon.mock_txpool_hashes("tests/rpc_resources/txpools/hashes_with_payment.json");
    let _transactions_mock = mock_daemon.mock_transactions(
        "tests/rpc_resources/transactions/hashes_with_payment.json",
        "tests/rpc_resources/transactions/txs_with_payment.json",
    );

    // There shouldn't be any update.
    subscriber
        .recv_timeout(Duration::from_secs(1))
        .await
        .expect_err("timeout waiting for invoice update");
}

#[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
#[test_case(InMemory::new(); "in-memory")]
#[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
#[tokio::test]
async fn track_parallel_invoices<S>(store: S)
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
    .expect("failed to build payment gateway");

    // Run it.
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
        .recv_timeout(Duration::from_secs(120))
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
        .recv_timeout(Duration::from_secs(120))
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
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");

    // Check that it is as expected.
    expected_1.amount_paid = 37419570;
    expected_1.assert_eq(&update);

    // Get update.
    let update = subscriber_2
        .recv_timeout(Duration::from_secs(120))
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
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");
    assert_eq!(update.amount_paid(), 0);
    let update = subscriber_2
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");
    assert_eq!(update.amount_paid(), 0);

    // Move forward a few blocks.
    for height in 2477658..2477663 {
        let height_mock = mock_daemon.mock_daemon_height(height);

        let update = subscriber_1
            .recv_timeout(Duration::from_secs(120))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        expected_1.expires_in = 2477664 - height;
        expected_1.current_height = height;
        expected_1.assert_eq(&update);

        let update = subscriber_2
            .recv_timeout(Duration::from_secs(120))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");

        expected_2.expires_in = 2477664 - height;
        expected_2.current_height = height;
        expected_2.assert_eq(&update);

        assert!(height_mock.hits() > 0);
    }

    // Put second payment in txpool.
    let txpool_hashes_mock =
        mock_daemon.mock_txpool_hashes("tests/rpc_resources/txpools/hashes_with_payment_2.json");
    let txpool_transactions_mock = mock_daemon.mock_txpool_transactions(
        "tests/rpc_resources/transactions/hashes_with_payment_2.json",
        "tests/rpc_resources/transactions/txs_with_payment_2.json",
    );

    // Invoice 1 should be paid now.
    let update = subscriber_1
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");

    expected_1.amount_paid = 74839140;
    expected_1.confirmations = Some(0);
    expected_1.assert_eq(&update);

    // Invoice 2 should not have an update.
    subscriber_2
        .recv_timeout(Duration::from_secs(1))
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
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");
    subscriber_2
        .recv_timeout(Duration::from_secs(1))
        .await
        .expect_err("should not have received an update, but did");
    let height_mock = mock_daemon.mock_daemon_height(2477663);

    let update = subscriber_1
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");

    expected_1.confirmations = Some(1);
    expected_1.expires_in = 1;
    expected_1.current_height = 2477663;
    expected_1.assert_eq(&update);

    let update = subscriber_2
        .recv_timeout(Duration::from_secs(120))
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
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");

    expected_1.confirmations = Some(2);
    expected_1.is_confirmed = true;
    expected_1.expires_in = 0;
    expected_1.current_height = 2477664;
    expected_1.assert_eq(&update);

    let update = subscriber_2
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");

    expected_2.expires_in = 0;
    expected_2.is_expired = true;
    expected_2.current_height = 2477664;
    expected_2.assert_eq(&update);

    assert!(txpool_hashes_mock.hits() > 0);
    assert!(height_mock.hits() > 0);
}

#[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
#[test_case(InMemory::new(); "in-memory")]
#[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
#[tokio::test]
async fn set_initial_height<S>(store: S)
where
    S: Storage + 'static,
{
    // Setup.
    init_logger();
    let mock_daemon = MockDaemon::new_mock_daemon().await;

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway_with_height = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        store,
    )
    // Faster scan rate so the update is received sooner.
    .scan_interval(Duration::from_millis(100))
    .daemon_url(mock_daemon.url(""))
    .account_index(1)
    .initial_height(2477657)
    .seed(1)
    .build()
    .expect("failed to build payment gateway");

    // Create payment gateway pointing at temp directory and mock daemon.
    let payment_gateway = PaymentGatewayBuilder::new(
        PRIVATE_VIEW_KEY.to_string(),
        PRIMARY_ADDRESS.to_string(),
        InMemory::new(),
    )
    // Faster scan rate so the update is received sooner.
    .scan_interval(Duration::from_millis(100))
    .daemon_url(mock_daemon.url(""))
    .account_index(1)
    .seed(1)
    .build()
    .expect("failed to build payment gateway");

    let _height_mock = mock_daemon.mock_daemon_height(2477664);

    // Run it.
    payment_gateway_with_height
        .run()
        .await
        .expect("failed to run payment gateway");
    payment_gateway
        .run()
        .await
        .expect("failed to run payment gateway");

    // Add the invoice.
    let invoice_id = payment_gateway_with_height
        .new_invoice(70000000, 2, 7, "invoice 1".to_string())
        .expect("failed to add new invoice to payment gateway for tracking");
    let mut subscriber_1 = payment_gateway_with_height
        .subscribe(invoice_id)
        .expect("invoice does not exist");

    // Get initial update.
    let update = subscriber_1
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");

    let mut expected_1 = MockInvoice::new(
        Some(update.address().to_string()),
        SubIndex::new(1, 97),
        2477664,
        70000000,
        2,
        7,
        "invoice 1".to_string(),
    );
    expected_1.current_height = 2477658;
    expected_1.expiration_height = 2477671;

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
        .recv_timeout(Duration::from_secs(120))
        .await
        .expect("timeout waiting for invoice update")
        .expect("subscription channel is closed");

    let mut expected_2 = expected_1.clone();
    expected_2.current_height = 2477664;
    expected_2.description = "invoice 2".to_string();

    // Check that it is as expected.
    expected_2.assert_eq(&update);

    for height in 2477659..2477665 {
        let update = subscriber_1
            .recv_timeout(Duration::from_secs(120))
            .await
            .expect("timeout waiting for invoice update")
            .expect("subscription channel is closed");
        assert_eq!(update.current_height(), height);
        assert_eq!(update.amount_paid(), 0);
    }
}
