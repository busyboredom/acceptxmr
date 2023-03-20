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
//! hopefully facilitate a speedy diagnosis an correction.
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
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use acceptxmr::{PaymentGateway, storage::stores::InMemory};
//! use std::time::Duration;
//!
//! let private_view_key =
//!     "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
//! let primary_address =
//!     "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";
//!
//! let store = InMemory::new();
//!
//! let payment_gateway = PaymentGateway::builder(
//!     private_view_key.to_string(),
//!     primary_address.to_string(),
//!     store
//! )
//! .scan_interval(Duration::from_millis(100)) // Scan for updates every 100 ms.
//! .build()?;
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
//! invoice storage implementation.
//!
//! ### `sled`
//!
//! The `sled` feature enables the [`Sled`](storage::stores::Sled) invoice
//! storage implementation. The `bincode` feature will also be enabled by this
//! feature.
//!
//! ### `sqlite`
//!
//! The `sqlite` feature enables the [`Sqlite`](storage::stores::Sqlite) invoice
//! storage implementation. The `bincode` feature will also be enabled by this
//! feature.

#![warn(clippy::pedantic)]
#![warn(missing_docs)]
#![warn(clippy::cargo)]
#![warn(clippy::panic)]
#![warn(clippy::unwrap_used)]
#![warn(clippy::expect_used)]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::module_name_repetitions)]

mod caching;
mod invoice;
mod payment_gateway;
mod pubsub;
mod rpc;
mod scanner;
pub mod storage;

use std::fmt::Debug;

pub use invoice::{Invoice, InvoiceId, SubIndex};
pub use payment_gateway::{PaymentGateway, PaymentGatewayBuilder, PaymentGatewayStatus};
pub use pubsub::{Subscriber, SubscriberError};
use rpc::RpcError;
use thiserror::Error;

/// Library's custom error type.
#[derive(Error, Debug)]
pub enum AcceptXmrError<E> {
    /// An error originating from a daemon RPC call.
    #[error("RPC error: {0}")]
    Rpc(#[from] RpcError),
    /// An error storing/retrieving [`Invoice`](crate::Invoice)s.
    #[error("invoice storage error: {0}")]
    InvoiceStorage(E),
    /// [`Subscriber`](crate::Subscriber) failed to retrieve update.
    #[error("subscriber failed to receive update: {0}")]
    Subscriber(#[from] SubscriberError),
    /// Failure to unblind the amount of an owned output.
    #[error("unable to unblind amount of owned output sent to subaddress index {0}")]
    Unblind(SubIndex),
    /// Failure to parse private view key.
    #[error("failed to parse {datatype} from \"{input}\": {error}")]
    Parse {
        /// Type to parse.
        datatype: &'static str,
        /// Input to parse.
        input: String,
        /// Error encountered.
        error: String,
    },
    /// Failure to check if output is owned.
    #[error("failed to check if output is owned: {0}")]
    OwnedOutputCheck(#[from] monero::blockdata::transaction::Error),
    /// Scanning thread exited with panic.
    #[error("scanning thread exited with panic")]
    ScanningThreadPanic,
    /// Payment gateway is already running.
    #[error("payment gateway is already running")]
    AlreadyRunning,
    /// Payment gateway encountered an error while creating scanning thread.
    #[error("payment gateway encountered an error while creating scanning thread: {0}")]
    Threading(#[from] std::io::Error),
    /// Payment gateway could not be stopped because the stop signal was not
    /// sent.
    #[error("payment gateway could not be stopped because the stop signal was not sent: {0}")]
    StopSignal(String),
}
