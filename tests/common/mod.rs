use std::ops::Deref;
use std::sync::Mutex;
use std::{env, fs};

use httpmock::{Mock, MockServer};
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};

pub const PRIVATE_VIEW_KEY: &str =
    "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
pub const PUBLIC_SPEND_KEY: &str =
    "7388a06bd5455b793a82b90ae801efb9cc0da7156df8af1d5800e4315cc627b4";

pub fn new_temp_dir() -> TempDir {
    Builder::new()
        .prefix("temp_db_")
        .rand_bytes(16)
        .tempdir()
        .expect("failed to generate temporary directory")
}

pub fn init_logger() {
    env::set_var(
            "RUST_LOG",
            "trace,mio=debug,want=debug,reqwest=info,sled=info,hyper=info,tracing=debug,httpmock=info,isahc=info",
        );
    let _ = env_logger::builder().is_test(true).try_init();
}

pub struct MockDaemon {
    server: MockServer,
    daemon_height_id: Mutex<Option<usize>>,
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
            txpool_id: Mutex::new(None),
            txpool_hashes_id: Mutex::new(None),
            txpool_transactions_id: Mutex::new(None),
        };
        // Mock daemon height request.
        mock_daemon.mock_daemon_height(2477657);
        // Mock txpool request.
        mock_daemon.mock_txpool("tests/rpc_resources/txpool.json");

        // Mock blocks.
        for i in 2477647..2477666 {
            // Mock block requests.
            let response_path = "tests/rpc_resources/".to_owned() + &i.to_string() + "/block.json";
            mock_daemon.mock_block(i, &response_path);

            // Skip block 2477661 when mocking transactions, because it has none.
            if i == 2477661 {
                continue;
            }

            // Mock block transaction requests.
            let request_path =
                "tests/rpc_resources/".to_owned() + &i.to_string() + "/txs_hashes_0.json";
            let response_path =
                "tests/rpc_resources/".to_owned() + &i.to_string() + "/transactions_0.json";
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

    pub fn mock_block(&self, height: u64, response_path: &str) -> Mock {
        self.mock(|when, then| {
            when.path("/json_rpc").body(
                r#"{"jsonrpc":"2.0","id":"0","method":"get_block","params":{"height":"#.to_owned()
                    + &height.to_string()
                    + "}}",
            );
            then.status(200)
                .header("content-type", "application/json")
                .body_from_file(response_path);
        })
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
