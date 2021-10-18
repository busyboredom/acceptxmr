//! # `AcceptXMR`: A Library for Accepting Monero
//!
//! This library aims to provide a simple, reliable, and efficient means to track monero payments.
//!
//! To track a payments, the [`PaymentGateway`] generates subaddresses using your private view key and
//! public spend key. It then watches for monero sent to that subaddress by periodically querying a
//! monero daemon of your choosing, and scanning newly received transactions for relevant outputs
//! using your private view key and public spend key.
//!
//! ## Security
//!
//! `AcceptXMR` is non-custodial, and does not require a hot wallet. However, it does require your
//! private view key and public spend key for scanning outputs. If keeping these private is important
//! to you, please take appropriate precautions to secure the platform you run your application on.
//!
//! Also note that anonymity networks like TOR are not currently supported for RPC calls. This
//! means that your network traffic will reveal that you are interacting with the monero network.
//!
//! ## Reliability
//!
//! This library strives for reliability, but that attempt may not be successful. `AcceptXMR` is
//! young and unproven, and relies on several crates which are undergoing rapid changes themselves
//! (for example, the database used ([Sled](sled)) is still in beta).
//!
//! That said, this payment gateway should survive unexpected power loss thanks to pending payments
//! being stored in a database, which is flushed to disk each time new blocks/transactions are
//! scanned. A best effort is made to keep the scanning thread free any of potential panics, and RPC
//! calls in the scanning thread are logged on failure and repeated next scan. In the event that an
//! error does occur, the liberal use of logging within this library will hopefully facilitate a
//! speedy diagnosis an correction.
//!
//! ## Performance
//!
//! For maximum performance, host your own monero daemon on the same local network. Network and
//! daemon slowness are primary cause of high payment update latency in the majority of use cases.
//!
//! To reduce the average latency before receiving payment updates, you may also consider lowering
//! the [`PaymentGateway`]'s `scan_interval` below the default of 1 second:
//! ```
//! use acceptxmr::PaymentGateway;
//! use std::time::Duration;
//!
//! let private_viewkey = "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
//! let public_spendkey = "7388a06bd5455b793a82b90ae801efb9cc0da7156df8af1d5800e4315cc627b4";
//!
//! let payment_gateway = PaymentGateway::builder(private_viewkey, public_spendkey)
//!     .scan_interval(Duration::from_millis(100)) // Scan for payment updates every 100 ms.
//!     .build();
//! ```
//!
//! Please note that `scan_interval` is the minimum time between scanning for updates. If your
//! daemon's response time is already greater than your `scan_interval`, or if your CPU is unable to
//! scan new transactions fast enough, reducing your `scan_interval` will do nothing.

#![warn(clippy::pedantic)]
#![warn(missing_docs)]
#![warn(clippy::cargo)]
#![allow(clippy::multiple_crate_versions)]

mod block_cache;
mod payments_db;
mod rpc;
mod scanner;
mod subaddress_cache;
mod subscriber;
mod txpool_cache;
mod util;

use std::cmp;
use std::cmp::Ordering;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::{atomic, Arc, Mutex, PoisonError};
use std::time::Duration;
use std::{fmt, thread, u64};

use log::{debug, info, warn};
use monero::cryptonote::subaddress;
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;
use tokio::{join, time};

use block_cache::BlockCache;
use payments_db::PaymentsDb;
use rpc::RpcClient;
use scanner::Scanner;
use subaddress_cache::SubaddressCache;
pub use subscriber::Subscriber;
use txpool_cache::TxpoolCache;
pub use util::AcceptXmrError;

const DEFAULT_SCAN_INTERVAL: Duration = Duration::from_millis(1000);
const DEFAULT_DAEMON: &str = "http://node.moneroworld.com:18089";
const DEFAULT_DB_PATH: &str = "AcceptXMR_DB";
const DEFAULT_RPC_CONNECTION_TIMEOUT: Duration = Duration::from_millis(2000);

/// The `PaymentGateway` allows you to track new [Payments](Payment), remove old payments from tracking, and
/// subscribe to payments that are already pending.
#[derive(Clone)]
pub struct PaymentGateway(pub(crate) Arc<PaymentGatewayInner>);

#[doc(hidden)]
pub struct PaymentGatewayInner {
    rpc_client: RpcClient,
    viewpair: monero::ViewPair,
    scan_interval: Duration,
    payments_db: PaymentsDb,
    subaddresses: Mutex<SubaddressCache>,
    height: Arc<atomic::AtomicU64>,
}

impl Deref for PaymentGateway {
    type Target = PaymentGatewayInner;

    fn deref(&self) -> &PaymentGatewayInner {
        &self.0
    }
}

impl PaymentGateway {
    /// Returns a builder used to create a new payment gateway.
    #[must_use]
    pub fn builder(private_viewkey: &str, public_spendkey: &str) -> PaymentGatewayBuilder {
        PaymentGatewayBuilder::new(private_viewkey, public_spendkey)
    }

    /// Runs the payment gateway. This function spawns a new thread, which periodically scans new
    /// blocks and transactions from the configured daemon and updates pending [Payments](Payment)
    /// in the database.
    ///
    /// # Errors
    ///
    /// Returns an [`AcceptXmrError::PaymentStorage`] error if there was an underlying issue with
    /// the database, or an [`AcceptXmrError::Rpc`] error if there was an issue getting necessary
    /// data from the monero daemon.
    ///
    /// # Panics
    ///
    /// This thread panics if successfully called more than once. Only one payment gateway should be
    /// running at a time. Note that if the first call resulted in an error, this can safely be
    /// called a second time.
    pub async fn run(&self, cache_size: u64) -> Result<(), AcceptXmrError> {
        // Gather info needed by the scanner.
        let rpc_client = self.rpc_client.clone();
        let viewpair = monero::ViewPair {
            view: self.viewpair.view,
            spend: self.viewpair.spend,
        };
        let scan_interval = self.scan_interval;
        let atomic_height = self.height.clone();
        let pending_payments = self.payments_db.clone();

        // Create scanner.
        let mut scanner = Scanner::new(
            rpc_client,
            viewpair,
            pending_payments,
            cache_size,
            atomic_height,
        )
        .await?;

        // Spawn the scanning thread.
        info!("Starting blockchain scanner");
        thread::Builder::new()
            .name("Scanning Thread".to_string())
            .spawn(move || {
                // The thread needs a tokio runtime to process async functions.
                let tokio_runtime = Runtime::new().unwrap();
                tokio_runtime.block_on(async move {
                    // Scan for transactions once every scan_interval.
                    let mut blockscan_interval = time::interval(scan_interval);
                    loop {
                        join!(blockscan_interval.tick(), scanner.scan());
                    }
                });
            })
            .expect("Error spawning scanning thread");
        Ok(())
    }

    /// Adds a new [Payment] to the payment gateway for tracking, and returns a [Subscriber] for
    /// receiving updates to that payment as they occur.
    ///
    /// # Errors
    ///
    /// Returns an error if there are any underlying issues modifying data in the
    /// database.
    ///
    /// # Panics
    ///
    /// Panics if `xmr` is negative, or larger than `u64::MAX`.
    pub async fn new_payment(
        &self,
        xmr: f64,
        confirmations_required: u64,
        expiration_in: u64,
    ) -> Result<Subscriber, AcceptXmrError> {
        // Convert xmr to picos.
        let amount = monero::Amount::from_xmr(xmr)
            .expect("amount due must be positive and less than u64::MAX")
            .as_pico();

        // Get subaddress in base58, and subaddress index.
        let (sub_index, subaddress) = self.subaddresses.lock().unwrap().remove_random();

        // Create payment object.
        let payment = Payment::new(
            &subaddress,
            sub_index,
            self.height.load(atomic::Ordering::Relaxed),
            amount,
            confirmations_required,
            expiration_in,
        );

        // Insert payment into database for tracking.
        self.payments_db.insert(&payment)?;
        debug!("Now tracking payment to subaddress index {}", payment.index);

        // Return a subscriber so the caller can get updates on payment status.
        // TODO: Consider not returning before a flush happens (maybe optionally flush when called?).
        Ok(self.subscribe(sub_index))
    }

    /// Remove (i.e. stop tracking) payment, returning the old payment if it existed.
    ///
    /// # Errors
    ///
    /// Returns an error if there are any underlying issues modifying/retrieving data in the
    /// database.
    pub fn remove_payment(&self, sub_index: SubIndex) -> Result<Option<Payment>, AcceptXmrError> {
        match self.payments_db.remove(sub_index)? {
            Some(old) => {
                if !(old.is_expired() || old.is_confirmed() && old.started_at < old.current_height)
                {
                    warn!("Removed a payment which was neither expired, nor fully confirmed and a block or more old. Was this intentional?");
                }
                // Put the subaddress back in the subaddress cache.
                self.subaddresses
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(sub_index, old.address.clone());

                Ok(Some(old))
            }
            None => Ok(None),
        }
    }

    /// Returns a `Subscriber` for the given subaddress index. If a tracked payment exists for that
    /// subaddress, the subscriber can be used to receive updates to for that payment.
    ///
    /// To subscribe to all payment updates, use the index of the primary address: (0,0).
    #[must_use]
    pub fn subscribe(&self, sub_index: SubIndex) -> Subscriber {
        self.payments_db.subscribe(sub_index)
    }

    /// Get current height of daemon using a monero daemon RPC call.
    ///
    /// # Errors
    ///
    /// Returns and error if a connection can not be made to the daemon, or if the daemon's response
    /// cannot be parsed.
    pub async fn daemon_height(&self) -> Result<u64, AcceptXmrError> {
        Ok(self.rpc_client.daemon_height().await?)
    }
}

/// A builder for the payment gateway. Used to configure your desired monero daemon, scan interval,
/// view key, etc.
pub struct PaymentGatewayBuilder {
    daemon_url: String,
    private_viewkey: monero::PrivateKey,
    public_spendkey: monero::PublicKey,
    scan_interval: Duration,
    db_path: String,
}

impl PaymentGatewayBuilder {
    /// Create a new payment gateway builder.
    #[must_use]
    pub fn new(private_viewkey: &str, public_spendkey: &str) -> PaymentGatewayBuilder {
        PaymentGatewayBuilder {
            daemon_url: DEFAULT_DAEMON.to_string(),
            private_viewkey: monero::PrivateKey::from_str(private_viewkey)
                .expect("invalid private viewkey"),
            public_spendkey: monero::PublicKey::from_str(public_spendkey)
                .expect("invalid public spendkey"),
            scan_interval: DEFAULT_SCAN_INTERVAL,
            db_path: DEFAULT_DB_PATH.to_string(),
        }
    }

    /// Set the url and port of your preferred monero daemon. Defaults to
    /// [http://node.moneroworld.com:18089](http://node.moneroworld.com:18089)
    #[must_use]
    pub fn daemon_url(mut self, url: &str) -> PaymentGatewayBuilder {
        reqwest::Url::parse(url).expect("invalid daemon URL");
        self.daemon_url = url.to_string();
        self
    }

    /// Set the minimum scan interval. New blocks / transactions will be scanned for relevant outputs
    /// at most every `interval`. Defaults to 1 second.
    #[must_use]
    pub fn scan_interval(mut self, interval: Duration) -> PaymentGatewayBuilder {
        self.scan_interval = interval;
        self
    }

    /// Path to the pending payments database. Defaults to `AcceptXMR_DB/`.
    #[must_use]
    pub fn db_path(mut self, path: &str) -> PaymentGatewayBuilder {
        self.db_path = path.to_string();
        self
    }

    /// Build the payment gateway.
    ///
    /// # Panics
    ///
    /// Panics the database cannot be opened at the path specified, or if the RPC client cannot load
    /// the system configuration or initialize a TLS backend.
    #[must_use]
    pub fn build(self) -> PaymentGateway {
        let rpc_client = RpcClient::new(&self.daemon_url, DEFAULT_RPC_CONNECTION_TIMEOUT)
            .expect("failed to create RPC client during PaymentGateway creation");
        let payments_db =
            PaymentsDb::new(&self.db_path).expect("failed to open pending payments database tree");
        info!("Opened database in \"{}/\"", self.db_path);
        let viewpair = monero::ViewPair {
            view: self.private_viewkey,
            spend: self.public_spendkey,
        };
        let subaddresses = SubaddressCache::init(&payments_db, viewpair);
        debug!("Generated {} initial subaddresses", subaddresses.len());

        PaymentGateway(Arc::new(PaymentGatewayInner {
            rpc_client,
            viewpair,
            scan_interval: self.scan_interval,
            payments_db,
            subaddresses: Mutex::new(subaddresses),
            height: Arc::new(atomic::AtomicU64::new(0)),
        }))
    }
}

/// Representation of a customer's payment. `Payment`s are created by the [`PaymentGateway`], and are
/// initially unpaid.
///
/// `Payment`s have an expiration block, after which they are considered expired. However, note that
/// the payment gateway by default will continue updating payments even after expiration.
///
/// To receive updates for a given `Payment`, use a [Subscriber].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Payment {
    address: String,
    index: SubIndex,
    started_at: u64,
    amount_requested: u64,
    amount_paid: u64,
    paid_at: Option<u64>,
    confirmations_required: u64,
    current_height: u64,
    expiration_at: u64,
    transfers: Vec<Transfer>,
}

impl Payment {
    fn new(
        address: &str,
        index: SubIndex,
        started_at: u64,
        amount_requested: u64,
        confirmations_required: u64,
        expiration_in: u64,
    ) -> Payment {
        let expiration_at = started_at + expiration_in;
        Payment {
            address: address.to_string(),
            index,
            started_at,
            amount_requested,
            amount_paid: 0,
            paid_at: None,
            confirmations_required,
            current_height: 0,
            expiration_at,
            transfers: Vec::new(),
        }
    }

    /// Returns `true` if the `Payment` has received the required number of confirmations.
    #[must_use]
    pub fn is_confirmed(&self) -> bool {
        self.confirmations().map_or(false, |confirmations| {
            confirmations >= self.confirmations_required
        })
    }

    /// Returns `true` if the `Payment`'s current block is greater than or equal to its expiration
    /// block.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        // At or passed the expiration block, AND not paid in full.
        self.current_height >= self.expiration_at && self.paid_at.is_none()
    }

    /// Returns the base 58 encoded subaddress of this `Payment`.
    #[must_use]
    pub fn address(&self) -> String {
        self.address.clone()
    }

    /// Returns the [subaddress index](SubIndex) of this `Payment`.
    #[must_use]
    pub fn index(&self) -> SubIndex {
        self.index
    }

    /// Returns the blockchain height at which the `Payment` was created.
    #[must_use]
    pub fn started_at(&self) -> u64 {
        self.started_at
    }

    /// Returns the amount of monero requested, in piconeros.
    #[must_use]
    pub fn amount_requested(&self) -> u64 {
        self.amount_requested
    }

    /// Returns the amount of monero paid, in piconeros.
    #[must_use]
    pub fn amount_paid(&self) -> u64 {
        self.amount_paid
    }

    /// Returns the number of confirmations this `Payment` requires before it is considered fully confirmed.
    #[must_use]
    pub fn confirmations_required(&self) -> u64 {
        self.confirmations_required
    }

    /// Returns the number of confirmations this `Payment` has received since it was paid in full.
    /// Returns None if the `Payment` has not yet been paid in full.
    #[must_use]
    pub fn confirmations(&self) -> Option<u64> {
        self.paid_at
            .map(|paid_at| self.current_height.saturating_sub(paid_at) + 1)
    }

    /// Returns the last height at which this `payment` was updated.
    #[must_use]
    pub fn current_height(&self) -> u64 {
        self.current_height
    }

    /// Returns the height at which this `Payment` will expire.
    #[must_use]
    pub fn expiration_at(&self) -> u64 {
        self.expiration_at
    }
}

impl fmt::Display for Payment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let confirmations = match self.confirmations() {
            Some(height) => height.to_string(),
            None => "N/A".to_string(),
        };
        let mut str = format!(
            "Index {}: \
            \nPaid: {}/{} \
            \nConfirmations: {} \
            \nStarted at: {} \
            \nCurrent height: {} \
            \nExpiration at: {} \
            \ntransfers: \
            \n[",
            self.index,
            monero::Amount::from_pico(self.amount_paid).as_xmr(),
            monero::Amount::from_pico(self.amount_requested).as_xmr(),
            confirmations,
            self.started_at,
            self.current_height,
            self.expiration_at,
        );
        for transfer in &self.transfers {
            let height = match transfer.height {
                Some(h) => h.to_string(),
                None => "N/A".to_string(),
            };
            str.push_str(&format!(
                "\n   {{Amount: {}, Height: {:?}}}",
                transfer.amount, height
            ));
        }
        if self.transfers.is_empty() {
            str.push(']');
        } else {
            str.push_str("\n]");
        }
        write!(f, "{}", str)
    }
}

/// A subaddress index.
#[derive(Debug, Copy, Clone, Hash, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubIndex {
    /// Subadress major index.
    pub major: u32,
    /// Subaddress minor index.
    pub minor: u32,
}

impl SubIndex {
    /// Create a new subaddress index from major and minor indexes.
    #[must_use]
    pub fn new(major: u32, minor: u32) -> SubIndex {
        SubIndex { major, minor }
    }
}

impl Ord for SubIndex {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.major.cmp(&other.major) {
            Ordering::Equal => self.minor.cmp(&other.minor),
            Ordering::Greater => Ordering::Greater,
            Ordering::Less => Ordering::Less,
        }
    }
}

impl PartialOrd for SubIndex {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for SubIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "{}/{}", self.major, self.minor)
    }
}

impl From<subaddress::Index> for SubIndex {
    fn from(index: subaddress::Index) -> SubIndex {
        SubIndex {
            major: index.major,
            minor: index.minor,
        }
    }
}

impl From<SubIndex> for subaddress::Index {
    fn from(index: SubIndex) -> subaddress::Index {
        subaddress::Index {
            major: index.major,
            minor: index.minor,
        }
    }
}

/// A `Transfer` represents a sum of owned outputs at a given height. When part of a `Payment`, it
/// specifically represents the sum of owned outputs for that payment's subaddress, at a given
/// height.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Copy)]
pub struct Transfer {
    /// Amount transferred in piconeros.
    pub amount: u64,
    /// Block height of the transfer, or None if the outputs are in the txpool.
    pub height: Option<u64>,
}

impl Transfer {
    fn new(amount: u64, height: Option<u64>) -> Transfer {
        Transfer { amount, height }
    }

    fn newer_than(&self, other_height: u64) -> bool {
        match self.height {
            Some(h) => h > other_height,
            None => true,
        }
    }

    fn older_than(&self, other_height: u64) -> bool {
        match self.height {
            Some(h) => h < other_height,
            None => false,
        }
    }

    fn cmp_by_age(&self, other: &Self) -> cmp::Ordering {
        match self.height {
            Some(height) => match other.height {
                Some(other_height) => height.cmp(&other_height),
                None => cmp::Ordering::Less,
            },
            None => match other.height {
                Some(_) => cmp::Ordering::Greater,
                None => cmp::Ordering::Equal,
            },
        }
    }
}
