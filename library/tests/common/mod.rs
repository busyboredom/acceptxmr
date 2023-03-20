use std::{cmp::max, collections::HashMap, fs, ops::Deref, sync::Mutex, time::Duration};

use acceptxmr::{
    storage::{
        stores::{InMemory, Sled, Sqlite},
        Storage,
    },
    Invoice, PaymentGatewayBuilder, SubIndex,
};
use httpmock::{Mock, MockServer};
use log::LevelFilter;
use serde_json::{json, Value};
use tempfile::Builder;
use test_case::test_case;
use tokio::runtime::Runtime;

pub const PRIVATE_VIEW_KEY: &str =
    "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
pub const PRIMARY_ADDRESS: &str =
    "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";

pub fn new_temp_dir() -> String {
    Builder::new()
        .prefix("temp_db_")
        .rand_bytes(16)
        .tempdir()
        .expect("failed to generate temporary directory")
        .path()
        .to_str()
        .expect("failed to get temporary directory path")
        .to_string()
}

pub fn init_logger() {
    let _ = env_logger::builder()
        .filter_level(LevelFilter::Warn)
        .filter_module("acceptxmr", log::LevelFilter::Trace)
        .is_test(true)
        .try_init();
}

#[derive(Clone)]
pub struct MockInvoice {
    pub address: Option<String>,
    pub index: SubIndex,
    pub creation_height: u64,
    pub amount_requested: u64,
    pub amount_paid: u64,
    pub paid_height: Option<u64>,
    pub confirmations_required: u64,
    pub current_height: u64,
    pub expiration_height: u64,
    pub description: String,

    // Calculated fields.
    pub is_expired: bool,
    pub expires_in: u64,
    pub is_confirmed: bool,
    pub confirmations: Option<u64>,
}

impl MockInvoice {
    pub fn new(
        address: Option<String>,
        index: SubIndex,
        creation_height: u64,
        amount_requested: u64,
        confirmations_required: u64,
        expires_in: u64,
        description: String,
    ) -> MockInvoice {
        MockInvoice {
            address,
            index,
            creation_height,
            amount_requested,
            amount_paid: 0,
            paid_height: None,
            confirmations_required,
            current_height: creation_height,
            expiration_height: creation_height + expires_in,
            description,

            is_expired: false,
            expires_in,
            is_confirmed: false,
            confirmations: None,
        }
    }

    pub fn assert_eq(&self, update: &Invoice) {
        if let Some(address) = &self.address {
            assert_eq!(update.address(), address);
        }
        assert_eq!(update.index(), self.index);
        assert_eq!(update.creation_height(), self.creation_height);
        assert_eq!(update.amount_requested(), self.amount_requested);
        assert_eq!(update.amount_paid(), self.amount_paid);
        assert_eq!(update.confirmations_required(), self.confirmations_required);
        assert_eq!(update.current_height(), self.current_height);
        assert_eq!(update.expiration_height(), self.expiration_height);
        assert_eq!(update.expiration_height(), self.expiration_height);
        assert_eq!(
            update.expiration_height() - max(update.creation_height(), update.current_height()),
            self.expires_in
        );
        assert_eq!(update.description(), self.description);

        // Calculated fields.
        assert_eq!(update.is_expired(), self.is_expired);
        assert_eq!(update.expiration_in(), self.expires_in);
        assert_eq!(update.is_confirmed(), self.is_confirmed);
        assert_eq!(update.confirmations(), self.confirmations);
    }
}

pub struct MockDaemon {
    server: MockServer,
    daemon_height_id: Mutex<Option<usize>>,
    block_ids: Mutex<HashMap<u64, usize>>,
    txpool_id: Mutex<Option<usize>>,
    txpool_hashes_id: Mutex<Option<usize>>,
    txpool_transactions_id: Mutex<Option<usize>>,
}

impl Deref for MockDaemon {
    type Target = MockServer;

    fn deref(&self) -> &MockServer {
        &self.server
    }
}

impl MockDaemon {
    pub fn new_mock_daemon() -> MockDaemon {
        let mock_daemon = MockDaemon {
            server: MockServer::start(),
            daemon_height_id: Mutex::new(None),
            block_ids: Mutex::new(HashMap::new()),
            txpool_id: Mutex::new(None),
            txpool_hashes_id: Mutex::new(None),
            txpool_transactions_id: Mutex::new(None),
        };
        // Mock daemon height request.
        mock_daemon.mock_daemon_height(2477657);
        // Mock txpool request.
        mock_daemon.mock_txpool("tests/rpc_resources/txpools/txpool.json");

        // Mock blocks.
        for i in 2477647..2477666 {
            // Mock block requests.
            let response_path =
                "tests/rpc_resources/blocks/".to_owned() + &i.to_string() + "/block.json";
            mock_daemon.mock_block(i, &response_path);

            // Skip block 2477661 when mocking transactions, because it has none.
            if i == 2477661 {
                continue;
            }

            // Mock block transaction requests.
            let request_path =
                "tests/rpc_resources/blocks/".to_owned() + &i.to_string() + "/txs_hashes_0.json";
            let response_path =
                "tests/rpc_resources/blocks/".to_owned() + &i.to_string() + "/transactions_0.json";
            mock_daemon.mock_transactions(&request_path, &response_path);
        }
        mock_daemon
    }

    pub fn mock_daemon_height(&self, height: u64) -> Mock {
        // Use mock ID to delete old daemon height mock.
        if let Some(id) = *self
            .daemon_height_id
            .lock()
            .expect("PoisonError when reading daemon height mock ID")
        {
            Mock::new(id, self).delete();
        };

        // Create the new daemon height mock.
        let mock = self.mock(|when, then| {
            when.path("/json_rpc")
                .body(r#"{"jsonrpc":"2.0","id":"0","method":"get_block_count"}"#);
            then.status(200)
                .header("content-type", "application/json")
                .json_body(json!({
                    "id": "0",
                    "jsonrpc": "2.0",
                    "result": {
                        "count": height,
                        "status": "OK"
                    }
                }));
        });
        *self
            .daemon_height_id
            .lock()
            .expect("PoisonError when writing daemon height mock ID") = Some(mock.id);
        mock
    }

    pub fn mock_alt_2477657(&self) {
        // Mock block requests.
        let response_path = "tests/rpc_resources/blocks/2477657_alt/block.json";
        self.mock_block(2477657, response_path);

        // Mock block transaction requests.
        let request_path = "tests/rpc_resources/blocks/2477657_alt/txs_hashes_0.json";
        let response_path = "tests/rpc_resources/blocks/2477657_alt/transactions_0.json";
        self.mock_transactions(request_path, response_path);
    }

    pub fn mock_alt_2477658(&self) {
        // Mock block requests.
        let response_path = "tests/rpc_resources/blocks/2477658_alt/block.json";
        self.mock_block(2477658, response_path);

        // Mock block transaction requests.
        let request_path = "tests/rpc_resources/blocks/2477658_alt/txs_hashes_0.json";
        let response_path = "tests/rpc_resources/blocks/2477658_alt/transactions_0.json";
        self.mock_transactions(request_path, response_path);
    }

    pub fn mock_txpool(&self, path: &str) -> Mock {
        // Use ID to delete old mock.
        if let Some(id) = *self
            .txpool_id
            .lock()
            .expect("PoisonError when reading txpool mock ID")
        {
            Mock::new(id, self).delete();
        };

        // Create new mock.
        let mock = self.mock(|when, then| {
            when.path("/get_transaction_pool").body("");
            then.status(200)
                .header("content-type", "application/json")
                .body_from_file(path);
        });
        *self
            .txpool_id
            .lock()
            .expect("PoisonError when writing txpool mock ID") = Some(mock.id);
        mock
    }

    pub fn mock_transactions(&self, request_path: &str, response_path: &str) -> Mock {
        let when_body: Value = serde_json::from_str(
            &fs::read_to_string(request_path)
                .expect("failed to read transaction request from file when preparing mock"),
        )
        .expect("failed to parse transaction request as json");
        let when_txs: Vec<&str> = when_body["txs_hashes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| v.as_str().expect("failed to parse tx hash as string"))
            .collect();
        self.mock(|when, then| {
            let mut when = when.path("/get_transactions");
            for hash in when_txs {
                // Ensure the request contains the hashes of all the expected transactions.
                when = when.body_contains(hash);
            }
            then.status(200)
                .header("content-type", "application/json")
                .body_from_file(response_path);
        })
    }

    pub fn mock_block(&self, height: u64, response_path: &str) {
        // Use ID to delete old mock.
        if let Some(id) = self
            .block_ids
            .lock()
            .expect("PoisonError when reading txpool mock ID")
            .get(&height)
        {
            Mock::new(*id, self).delete();
        };
        let mock = self.mock(|when, then| {
            when.path("/json_rpc").body(
                r#"{"jsonrpc":"2.0","id":"0","method":"get_block","params":{"height":"#.to_owned()
                    + &height.to_string()
                    + "}}",
            );
            then.status(200)
                .header("content-type", "application/json")
                .body_from_file(response_path);
        });
        self.block_ids
            .lock()
            .expect("PoisonError when writing daemon height mock ID")
            .insert(height, mock.id);
    }

    pub fn mock_txpool_hashes(&self, response_path: &str) -> Mock {
        // Use ID to delete old mock.
        if let Some(id) = *self
            .txpool_hashes_id
            .lock()
            .expect("PoisonError when reading txpool hashes mock ID")
        {
            Mock::new(id, self).delete();
        };

        // Create new mock.
        let mock = self.mock(|when, then| {
            when.path("/get_transaction_pool_hashes").body("");
            then.status(200)
                .header("content-type", "application/json")
                .body_from_file(response_path);
        });
        *self
            .txpool_hashes_id
            .lock()
            .expect("PoisonError when writing txpool hashes mock ID") = Some(mock.id);
        mock
    }

    pub fn mock_txpool_transactions(&self, request_path: &str, response_path: &str) -> Mock {
        // Use ID to delete old mock.
        if let Some(id) = *self
            .txpool_transactions_id
            .lock()
            .expect("PoisonError when reading txpool transactions mock ID")
        {
            Mock::new(id, self).delete();
        };
        let mock = self.mock_transactions(request_path, response_path);
        *self
            .txpool_transactions_id
            .lock()
            .expect("PoisonError when writing txpool transactions mock ID") = Some(mock.id);
        mock
    }
}

#[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
#[test_case(InMemory::new(); "in-memory")]
#[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
fn reproducible_rand<S>(store: S)
where
    S: Storage + 'static,
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
