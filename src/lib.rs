//! # `AcceptXMR`: A Library for Accepting Monero
//!
//! This library aims to provide a simple, reliable, and efficient means to track monero payments.
//!
//! To track payments, the [`PaymentGateway`] generates subaddresses using your private view key and
//! public spend key. It then watches for monero sent to that subaddress using a monero daemon of
//! your choosing, your private view key and your public spend key.
//!
//! ## Security
//!
//! `AcceptXMR` is non-custodial, and does not require a hot wallet. However, it does require your
//! private view key and public spend key for scanning outputs. If keeping these private is important
//! to you, please take appropriate precautions to secure the platform you run your application on
//! _and keep your private view key out of your git repository!_.
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
//! That said, this payment gateway should survive unexpected power loss thanks to pending invoices
//! being stored in a database, which is flushed to disk each time new blocks/transactions are
//! scanned. A best effort is made to keep the scanning thread free any of potential panics, and RPC
//! calls in the scanning thread are logged on failure and repeated next scan. In the event that an
//! error does occur, the liberal use of logging within this library will hopefully facilitate a
//! speedy diagnosis an correction.
//!
//! Use this library at your own risk.
//!
//! ## Performance
//!
//! For maximum performance, host your own monero daemon on the same local network. Network and
//! daemon slowness are primary cause of high invoice update latency in the majority of use cases.
//!
//! To reduce the average latency before receiving invoice updates, you may also consider lowering
//! the [`PaymentGateway`]'s `scan_interval` below the default of 1 second:
//! ```
//! # use tempfile::Builder;
//! use acceptxmr::PaymentGateway;
//! use std::time::Duration;
//!
//! # let temp_dir = Builder::new()
//! #   .prefix("temp_db_")
//! #   .rand_bytes(16)
//! #   .tempdir().expect("Failed to generate temporary directory");
//!
//! let private_view_key = "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
//! let public_spend_key = "7388a06bd5455b793a82b90ae801efb9cc0da7156df8af1d5800e4315cc627b4";
//!
//! let payment_gateway = PaymentGateway::builder(private_view_key, public_spend_key)
//!     .scan_interval(Duration::from_millis(100)) // Scan for invoice updates every 100 ms.
//! #   .db_path(temp_dir.path().to_str().expect("Failed to get temporary directory path"))
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

mod caching;
mod invoice;
mod invoices_db;
mod rpc;
mod scanner;
mod subscriber;
mod util;

use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, AtomicU64};
use std::sync::{atomic, Arc, Mutex, PoisonError};
use std::time::Duration;
use std::{thread, u64};

use log::{debug, info, warn};
use monero::cryptonote::onetime_key::SubKeyChecker;
use tokio::runtime::Runtime;
use tokio::{join, time};

use caching::{BlockCache, SubaddressCache, TxpoolCache};
pub use invoice::{Invoice, SubIndex};
use invoices_db::InvoicesDb;
use rpc::RpcClient;
use scanner::Scanner;
pub use subscriber::Subscriber;
pub use util::AcceptXmrError;

const DEFAULT_SCAN_INTERVAL: Duration = Duration::from_millis(1000);
const DEFAULT_DAEMON: &str = "http://node.moneroworld.com:18089";
const DEFAULT_DB_PATH: &str = "AcceptXMR_DB";
const DEFAULT_RPC_CONNECTION_TIMEOUT: Duration = Duration::from_millis(2000);
const DEFAULT_BLOCK_CACHE_SIZE: u64 = 10;

/// The `PaymentGateway` allows you to track new [`Invoice`s](Invoice), remove old `Invoice`s from tracking, and
/// subscribe to `Invoice`s that are already pending.
#[derive(Clone)]
pub struct PaymentGateway(pub(crate) Arc<PaymentGatewayInner>);

#[doc(hidden)]
pub struct PaymentGatewayInner {
    rpc_client: RpcClient,
    viewpair: monero::ViewPair,
    scan_interval: Duration,
    invoices_db: InvoicesDb,
    subaddresses: Mutex<SubaddressCache>,
    highest_minor_index: Arc<AtomicU32>,
    height: Arc<AtomicU64>,
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
    pub fn builder(private_view_key: &str, public_spend_key: &str) -> PaymentGatewayBuilder {
        PaymentGatewayBuilder::new(private_view_key, public_spend_key)
    }

    /// Runs the payment gateway. This function spawns a new thread, which periodically scans new
    /// blocks and transactions from the configured daemon and updates pending [`Invoice`s](Invoice)
    /// in the database.
    ///
    /// This method should only be called once.
    ///
    /// # Errors
    ///
    /// Returns an [`AcceptXmrError::InvoiceStorage`] error if there was an underlying issue with
    /// the database, or an [`AcceptXmrError::Rpc`] error if there was an issue getting necessary
    /// data from the monero daemon.
    #[allow(clippy::range_plus_one, clippy::missing_panics_doc)]
    pub async fn run(&self) -> Result<(), AcceptXmrError> {
        // Gather info needed by the scanner.
        let rpc_client = self.rpc_client.clone();
        let viewpair = self.viewpair;
        let scan_interval = self.scan_interval;
        let highest_minor_index = self.highest_minor_index.clone();
        let atomic_height = self.height.clone();
        let pending_invoices = self.invoices_db.clone();

        // Create scanner.
        let mut scanner = Scanner::new(
            rpc_client,
            pending_invoices,
            DEFAULT_BLOCK_CACHE_SIZE,
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
                    // Create persistent sub key checker for efficient tx output checking.
                    let mut sub_key_checker = SubKeyChecker::new(
                        &viewpair,
                        1..2,
                        0..highest_minor_index.load(atomic::Ordering::Relaxed) + 1,
                    );
                    // Scan for transactions once every scan_interval.
                    let mut blockscan_interval = time::interval(scan_interval);
                    loop {
                        // Update sub key checker if necessary.
                        if sub_key_checker.table.len()
                            <= highest_minor_index.load(atomic::Ordering::Relaxed) as usize
                        {
                            sub_key_checker = SubKeyChecker::new(
                                &viewpair,
                                1..2,
                                0..highest_minor_index.load(atomic::Ordering::Relaxed) + 1,
                            );
                        }
                        // Scan!
                        join!(blockscan_interval.tick(), scanner.scan(&sub_key_checker));
                    }
                });
            })
            .expect("Error spawning scanning thread");
        debug!("Scanner started successfully");
        Ok(())
    }

    /// Adds a new [`Invoice`] to the payment gateway for tracking, and returns a [Subscriber] for
    /// receiving updates to that invoice as they occur.
    ///
    /// # Errors
    ///
    /// Returns an error if there are any underlying issues modifying data in the
    /// database.
    pub async fn new_invoice(
        &self,
        piconeros: u64,
        confirmations_required: u64,
        expiration_in: u64,
    ) -> Result<Subscriber, AcceptXmrError> {
        let amount = piconeros;

        // Get subaddress in base58, and subaddress index.
        let (sub_index, subaddress) = self
            .subaddresses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove_random();

        // Create invoice object.
        let invoice = Invoice::new(
            &subaddress,
            sub_index,
            self.height.load(atomic::Ordering::Relaxed),
            amount,
            confirmations_required,
            expiration_in,
        );

        // Insert invoice into database for tracking.
        self.invoices_db.insert(&invoice)?;
        debug!(
            "Now tracking invoice to subaddress index {}",
            invoice.index()
        );

        // Return a subscriber so the caller can get updates on invoice status.
        // TODO: Consider not returning before a flush happens (maybe optionally flush when called?).
        Ok(self.subscribe(sub_index))
    }

    /// Remove (i.e. stop tracking) invoice, returning the old invoice if it existed.
    ///
    /// # Errors
    ///
    /// Returns an error if there are any underlying issues modifying/retrieving data in the
    /// database.
    pub fn remove_invoice(&self, sub_index: SubIndex) -> Result<Option<Invoice>, AcceptXmrError> {
        match self.invoices_db.remove(sub_index)? {
            Some(old) => {
                if !(old.is_expired()
                    || old.is_confirmed() && old.creation_height() < old.current_height())
                {
                    warn!("Removed an invoice which was neither expired, nor fully confirmed and a block or more old. Was this intentional?");
                }
                // Put the subaddress back in the subaddress cache.
                self.subaddresses
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(sub_index, old.address());

                Ok(Some(old))
            }
            None => Ok(None),
        }
    }

    /// Returns a `Subscriber` for the given subaddress index. If a tracked invoice exists for that
    /// subaddress, the subscriber can be used to receive updates to for that invoice.
    ///
    /// To subscribe to all invoice updates, use the index of the primary address: (0,0).
    #[must_use]
    pub fn subscribe(&self, sub_index: SubIndex) -> Subscriber {
        self.invoices_db.subscribe(sub_index)
    }

    /// Get current height of daemon using a monero daemon RPC call.
    ///
    /// # Errors
    ///
    /// Returns an error if a connection can not be made to the daemon, or if the daemon's response
    /// cannot be parsed.
    pub async fn daemon_height(&self) -> Result<u64, AcceptXmrError> {
        Ok(self.rpc_client.daemon_height().await?)
    }

    /// Returns URL of configured daemon.
    #[must_use]
    pub fn daemon_url(&self) -> String {
        self.rpc_client.url()
    }
}

/// A builder for the payment gateway. Used to configure your desired monero daemon, scan interval,
/// view key, etc.
///
/// # Examples
///
/// ```
/// # use tempfile::Builder;
/// use acceptxmr::PaymentGatewayBuilder;
/// use std::time::Duration;
///
/// # let temp_dir = Builder::new()
/// #   .prefix("temp_db_")
/// #   .rand_bytes(16)
/// #   .tempdir().expect("Failed to generate temporary directory");
///
/// let private_view_key = "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
/// let public_spend_key = "7388a06bd5455b793a82b90ae801efb9cc0da7156df8af1d5800e4315cc627b4";
///
/// // Create a payment gateway with an extra fast scan rate and a custom monero daemon URL.
/// let payment_gateway = PaymentGatewayBuilder::new(private_view_key, public_spend_key)
///     .scan_interval(Duration::from_millis(100)) // Scan for invoice updates every 100 ms.
///     .daemon_url("http://example.com:18081") // Set custom monero daemon URL.
/// #   .db_path(temp_dir.path().to_str().expect("Failed to get temporary directory path"))
///     .build();
/// ```
pub struct PaymentGatewayBuilder {
    daemon_url: String,
    private_view_key: monero::PrivateKey,
    public_spend_key: monero::PublicKey,
    scan_interval: Duration,
    db_path: String,
    seed: Option<u64>,
}

impl PaymentGatewayBuilder {
    /// Create a new payment gateway builder.
    #[must_use]
    pub fn new(private_view_key: &str, public_spend_key: &str) -> PaymentGatewayBuilder {
        PaymentGatewayBuilder {
            daemon_url: DEFAULT_DAEMON.to_string(),
            private_view_key: monero::PrivateKey::from_str(private_view_key)
                .expect("invalid private view key"),
            public_spend_key: monero::PublicKey::from_str(public_spend_key)
                .expect("invalid public spend key"),
            scan_interval: DEFAULT_SCAN_INTERVAL,
            db_path: DEFAULT_DB_PATH.to_string(),
            seed: None,
        }
    }

    /// Set the url and port of your preferred monero daemon. Defaults to
    /// [http://node.moneroworld.com:18089](http://node.moneroworld.com:18089).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// #
    /// use acceptxmr::PaymentGatewayBuilder;
    ///
    /// let private_view_key = "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
    /// let public_spend_key = "7388a06bd5455b793a82b90ae801efb9cc0da7156df8af1d5800e4315cc627b4";
    ///
    /// // Create a payment gateway with a custom monero daemon URL.
    /// let payment_gateway = PaymentGatewayBuilder::new(private_view_key, public_spend_key)
    ///     .daemon_url("http://example.com:18081") // Set custom monero daemon URL.
    ///     .build();
    ///
    /// // The payment gateway will now use the daemon specified.
    /// payment_gateway.run().await?;
    /// #   Ok(())
    /// # }
    /// ```
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

    /// Path to the pending invoices database. Defaults to `AcceptXMR_DB/`.
    #[must_use]
    pub fn db_path(mut self, path: &str) -> PaymentGatewayBuilder {
        self.db_path = path.to_string();
        self
    }

    /// Seed for random number generator. Use only for reproducible testing. Do not set in a
    /// production environment.
    #[must_use]
    pub fn seed(mut self, seed: u64) -> PaymentGatewayBuilder {
        warn!("Seed set to {}. Some operations intended to be random (like the order in which subaddresses are used) will be predictable.", seed);
        self.seed = Some(seed);
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
        let invoices_db =
            InvoicesDb::new(&self.db_path).expect("failed to open pending invoices database tree");
        info!("Opened database in \"{}/\"", self.db_path);
        let viewpair = monero::ViewPair {
            view: self.private_view_key,
            spend: self.public_spend_key,
        };
        let highest_minor_index = Arc::new(AtomicU32::new(0));
        let subaddresses = SubaddressCache::init(
            &invoices_db,
            viewpair,
            highest_minor_index.clone(),
            self.seed,
        );
        debug!("Generated {} initial subaddresses", subaddresses.len());

        PaymentGateway(Arc::new(PaymentGatewayInner {
            rpc_client,
            viewpair,
            scan_interval: self.scan_interval,
            invoices_db,
            subaddresses: Mutex::new(subaddresses),
            highest_minor_index,
            height: Arc::new(atomic::AtomicU64::new(0)),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use tempfile::{Builder, TempDir};

    use crate::PaymentGatewayBuilder;

    fn init_logger() {
        env::set_var(
            "RUST_LOG",
            "debug,mio=debug,want=debug,reqwest=info,sled=info,hyper=info,tracing=debug,httpmock=info,isahc=info",
        );
        let _ = env_logger::builder().is_test(true).try_init();
    }

    fn new_temp_dir() -> TempDir {
        Builder::new()
            .prefix("temp_db_")
            .rand_bytes(16)
            .tempdir()
            .expect("failed to generate temporary directory")
    }

    const PRIVATE_VIEW_KEY: &str =
        "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
    const PUBLIC_SPEND_KEY: &str =
        "7388a06bd5455b793a82b90ae801efb9cc0da7156df8af1d5800e4315cc627b4";

    #[test]
    fn test_daemon_url() {
        // Setup.
        init_logger();
        let temp_dir = new_temp_dir();

        let payment_gateway = PaymentGatewayBuilder::new(PRIVATE_VIEW_KEY, PUBLIC_SPEND_KEY)
            .db_path(
                temp_dir
                    .path()
                    .to_str()
                    .expect("failed to get temporary directory path"),
            )
            .daemon_url("http://example.com:18081")
            .build();

        assert_eq!(payment_gateway.rpc_client.url(), "http://example.com:18081");
    }
}
