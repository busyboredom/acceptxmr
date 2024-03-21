use std::{collections::HashMap, fs, ops::Deref, sync::Mutex};

use httpmock::{Mock, MockServer};
use serde_json::{json, Value};

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
    pub async fn new_mock_daemon() -> MockDaemon {
        let mock_daemon = MockDaemon {
            server: MockServer::start_async().await,
            daemon_height_id: Mutex::new(None),
            block_ids: Mutex::new(HashMap::new()),
            txpool_id: Mutex::new(None),
            txpool_hashes_id: Mutex::new(None),
            txpool_transactions_id: Mutex::new(None),
        };
        // Mock daemon height request.
        mock_daemon.mock_daemon_height(2_477_657);
        // Mock txpool request.
        mock_daemon.mock_txpool("../testing-utils/rpc_resources/txpools/txpool.json");
        // Mock txpool hashes.
        mock_daemon.mock_txpool_hashes("../testing-utils/rpc_resources/txpools/hashes.json");

        // Mock blocks.
        for i in 2_477_647..2_477_666 {
            // Mock block requests.
            let response_path = "../testing-utils/rpc_resources/blocks/".to_owned()
                + &i.to_string()
                + "/block.json";
            mock_daemon.mock_block(i, &response_path);

            // Skip block 2477661 when mocking transactions, because it has none.
            if i == 2_477_661 {
                continue;
            }

            // Mock block transaction requests.
            let request_path = "../testing-utils/rpc_resources/blocks/".to_owned()
                + &i.to_string()
                + "/txs_hashes_0.json";
            let response_path = "../testing-utils/rpc_resources/blocks/".to_owned()
                + &i.to_string()
                + "/transactions_0.json";
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
        let response_path = "../testing-utils/rpc_resources/blocks/2477657_alt/block.json";
        self.mock_block(2_477_657, response_path);

        // Mock block transaction requests.
        let request_path = "../testing-utils/rpc_resources/blocks/2477657_alt/txs_hashes_0.json";
        let response_path = "../testing-utils/rpc_resources/blocks/2477657_alt/transactions_0.json";
        self.mock_transactions(request_path, response_path);
    }

    pub fn mock_alt_2477658(&self) {
        // Mock block requests.
        let response_path = "../testing-utils/rpc_resources/blocks/2477658_alt/block.json";
        self.mock_block(2_477_658, response_path);

        // Mock block transaction requests.
        let request_path = "../testing-utils/rpc_resources/blocks/2477658_alt/txs_hashes_0.json";
        let response_path = "../testing-utils/rpc_resources/blocks/2477658_alt/transactions_0.json";
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
