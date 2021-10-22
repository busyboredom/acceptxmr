use std::{env, fs};

use httpmock::MockServer;
use log::trace;
use serde_json::{from_str, Value};
use tempfile::{Builder, TempDir};

pub const PRIVATE_VIEW_KEY: &str =
    "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
pub const PUBLIC_SPEND_KEY: &str =
    "7388a06bd5455b793a82b90ae801efb9cc0da7156df8af1d5800e4315cc627b4";

pub fn new_mock_daemon() -> MockServer {
    let mock_daemon = MockServer::start();
    // Mock daemon height request.
    mock_daemon.mock(|when, then| {
        when.path("/json_rpc")
            .body(r#"{"jsonrpc":"2.0","id":"0","method":"get_block_count"}"#);
        then.status(200)
            .header("content-type", "application/json")
            .body_from_file("tests/rpc_resources/2429479/daemon_height.json");
    });
    // Mock txpool request.
    mock_daemon.mock(|when, then| {
        when.path("/get_transaction_pool").body("");
        then.status(200)
            .header("content-type", "application/json")
            .body_from_file("tests/rpc_resources/txpool.json");
    });
    for i in 2429470..2429480 {
        // Mock block requests.
        mock_daemon.mock(|when, then| {
            when.path("/json_rpc").body(
                r#"{"jsonrpc":"2.0","id":"0","method":"get_block","params":{"height":"#.to_owned()
                    + &i.to_string()
                    + "}}",
            );
            then.status(200)
                .header("content-type", "application/json")
                .body_from_file("tests/rpc_resources/".to_owned() + &i.to_string() + "/block.json");
        });
        // Mock block transaction requests.
        mock_daemon.mock(|when, then| {
            let when_body = fs::read_to_string(
                "tests/rpc_resources/".to_owned() + &i.to_string() + "/txs_hashes.json",
            )
            .expect("failed to read transaction request from file when preparing mock");
            trace!("Building mock for request body: {}", when_body);

            when.path("/get_transactions")
                .json_body(from_str::<Value>(&when_body).expect("failed to parse file as json"));
            then.status(200)
                .header("content-type", "application/json")
                .body_from_file(
                    "tests/rpc_resources/".to_owned() + &i.to_string() + "/transactions.json",
                );
        });
    }
    mock_daemon
}

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
        "debug,mio=debug,want=debug,reqwest=info,sled=info,hyper=info,tracing=debug,httpmock=info,isahc=info",
    );
    env_logger::init();
}
