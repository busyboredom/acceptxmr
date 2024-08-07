//! # `AcceptXMR`: Accept Monero in Your Application
//!
//! This library aims to provide a simple, reliable, and efficient means to
//! track monero payments.
//!
//! To track payments, the [`PaymentGateway`] generates subaddresses using your
//! private view key and primary address. It then watches for monero sent to
//! that subaddress using a monero daemon of your choosing, your private view
//! key and your primary address.
//!
//! Use this library at your own risk, it is young and unproven.
//!
//! ## Key Advantages
//! * View pair only, no hot wallet.
//! * Subaddress based.
//! * Pending invoices can be stored persistently, enabling recovery from power
//!   loss.
//! * Number of confirmations is configurable per-invoice.
//! * Ignores transactions with non-zero timelocks.
//! * Payment can occur over multiple transactions.
//!
//! ## Security
//!
//! `AcceptXMR` is non-custodial, and does not require a hot wallet. However, it
//! does require your private view key and primary address for scanning outputs.
//! If keeping these private is important to you, please take appropriate
//! precautions to secure the platform you run your application on
//! _and keep your private view key out of your git repository!_.
//!
//! Care is taken to protect users from malicious transactions containing
//! timelocks or duplicate output keys (i.e. the burning bug). For the best
//! protection against the burning bug, it is recommended that users use a
//! dedicated wallet or account index for `AcceptXMR` that is not used for any
//! other purpose. The payment gateway's initial height should also be set to
//! the wallet's restore height. These measures allow `AcceptXMR` to keep a full
//! inventory of used output keys so that duplicates can be reliably identified.
//!
//! Also note that anonymity networks like TOR are not currently supported for
//! RPC calls. This means that your network traffic will reveal that you are
//! interacting with the monero network.
//!
//! ## Reliability
//!
//! This library strives for reliability, but that attempt may not be
//! successful. `AcceptXMR` is young and unproven, and relies on several crates
//! which are undergoing rapid changes themselves. For example, the primary
//! storage layer implementation ([`Sled`](https://docs.rs/sled)) is still in
//! beta.
//!
//! That said, this payment gateway should survive unexpected power loss thanks
//! to the ability to flush pending invoices to disk each time new
//! blocks/transactions are scanned. A best effort is made to keep the scanning
//! thread free any of potential panics, and RPC calls in the scanning
//! thread are logged on failure and repeated next scan. In the event that an
//! error does occur, the liberal use of logging within this library will
//! hopefully facilitate a speedy diagnosis and correction.
//!
//! Use this library at your own risk.
//!
//! ## Performance
//!
//! It is strongly recommended that you host your own monero daemon on the same
//! local network. Network and daemon slowness are the primary cause of high
//! invoice update latency in the majority of use cases.
//!
//! To reduce the average latency before receiving invoice updates, you may also
//! consider lowering the [`PaymentGateway`]'s `scan_interval` below the default
//! of 1 second:
//! ```
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use acceptxmr::{PaymentGatewayBuilder, storage::stores::InMemory};
//! use std::time::Duration;
//!
//! let private_view_key =
//!     "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
//! let primary_address =
//!     "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";
//!
//! let store = InMemory::new();
//!
//! let payment_gateway = PaymentGatewayBuilder::new(
//!     private_view_key.to_string(),
//!     primary_address.to_string(),
//!     store
//! )
//! .scan_interval(Duration::from_millis(100)) // Scan for updates every 100 ms.
//! .build()
//! .await?;
//! #   Ok(())
//! # }
//! ```
//!
//! Please note that `scan_interval` is the minimum time between scanning for
//! updates. If your daemon's response time is already greater than your
//! `scan_interval`, or if your CPU is unable to scan new transactions fast
//! enough, reducing your `scan_interval` will do nothing.
//!
//! ## Features
//!
//! ### `Serde`
//!
//! The `serde` feature enables `serde` (de)serialization on select types.
//!
//! ### `bincode`
//!
//! The `bincode` feature enables `bincode` (de)serialization on select types.
//!
//! ### `in-memory`
//!
//! The `in-memory` feature enables the [`InMemory`](storage::stores::InMemory)
//! storage implementation.
//!
//! ### `sled`
//!
//! The `sled` feature enables the [`Sled`](storage::stores::Sled) storage
//! implementation. The `bincode` feature will also be enabled by this feature.
//!
//! ### `sqlite`
//!
//! The `sqlite` feature enables the [`Sqlite`](storage::stores::Sqlite) storage
//! implementation. The `bincode` feature will also be enabled by this feature.

#![warn(clippy::panic)]
#![warn(clippy::unwrap_used)]
#![warn(clippy::expect_used)]
#![allow(clippy::multiple_crate_versions)]
// Show feature flag tags on `docs.rs`
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

mod caching;
mod invoice;
mod monerod_client;
mod payment_gateway;
mod pubsub;
mod scanner;
pub mod storage;

use std::fmt::Debug;

pub use invoice::{Invoice, InvoiceId, SubIndex};
pub use monerod_client::{
    Client as MonerodClient, MockClient as MonerodMockClient, RpcClient as MonerodRpcClient,
    RpcError,
};
pub use payment_gateway::{PaymentGateway, PaymentGatewayBuilder, PaymentGatewayStatus};
pub use pubsub::{Subscriber, SubscriberError};
use scanner::ScannerError;
use storage::StorageError;
use thiserror::Error;

/// Library's custom error type.
#[derive(Error, Debug)]
pub enum AcceptXmrError {
    /// An error originating from a monero daemon RPC call.
    #[error("Monerod RPC error: {0}")]
    Rpc(#[from] RpcError),
    /// An error storing/retrieving data from the storage layer.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    /// Failure to parse.
    #[error("failed to parse {datatype} from \"{input}\": {error}")]
    Parse {
        /// Type to parse.
        datatype: &'static str,
        /// Input to parse.
        input: String,
        /// Error encountered.
        error: String,
    },
    /// Blockchain scanner encountered an error.
    #[error("blockchain scanner encountered an error: {0}")]
    Scanner(#[from] ScannerError),
    /// Payment gateway is already running.
    #[error("payment gateway is already running")]
    AlreadyRunning,
    /// Payment gateway could not be stopped because the stop signal was not
    /// sent.
    #[error("payment gateway could not be stopped because the stop signal was not sent: {0}")]
    StopSignal(String),
}
