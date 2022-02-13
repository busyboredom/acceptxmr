use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{self, AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use hyper::Uri;
use log::{debug, info, warn};
use monero::cryptonote::onetime_key::SubKeyChecker;
use tokio::runtime::Runtime;
use tokio::{join, time};

use crate::caching::SubaddressCache;
use crate::invoices_db::InvoicesDb;
use crate::rpc::RpcClient;
use crate::scanner::Scanner;
use crate::subscriber::Subscriber;
use crate::{AcceptXmrError, Invoice, InvoiceId};

const DEFAULT_SCAN_INTERVAL: Duration = Duration::from_millis(1000);
const DEFAULT_DAEMON: &str = "http://node.moneroworld.com:18089";
const DEFAULT_DB_PATH: &str = "AcceptXMR_DB";
const DEFAULT_RPC_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_RPC_TOTAL_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_BLOCK_CACHE_SIZE: u64 = 10;

/// The `PaymentGateway` allows you to track new [`Invoice`](Invoice)s, remove old `Invoice`s from
/// tracking, and subscribe to `Invoice`s that are already pending.
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
    block_cache_height: Arc<AtomicU64>,
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
    /// blocks and transactions from the configured daemon and updates pending [`Invoice`](Invoice)s
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
        let block_cache_height = self.block_cache_height.clone();
        let pending_invoices = self.invoices_db.clone();

        // Create scanner.
        debug!("Creating blockchain scanner");
        let mut scanner = Scanner::new(
            rpc_client,
            pending_invoices,
            DEFAULT_BLOCK_CACHE_SIZE,
            block_cache_height,
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

    /// Adds a new [`Invoice`] to the payment gateway for tracking, and returns the ID of the new
    /// invoice. Use a [`Subscriber`] to receive updates on the new invoice invoice as they occur.
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
        description: &str,
    ) -> Result<InvoiceId, AcceptXmrError> {
        let amount = piconeros;

        // Get subaddress in base58, and subaddress index.
        let (sub_index, subaddress) = self
            .subaddresses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove_random();

        // Add one because the highest block is always one less than the daemon height.
        let creation_height = self.block_cache_height.load(atomic::Ordering::Relaxed) + 1;

        // Create invoice object.
        let invoice = Invoice::new(
            &subaddress,
            sub_index,
            creation_height,
            amount,
            confirmations_required,
            expiration_in,
            description,
        );

        // Insert invoice into database for tracking.
        self.invoices_db.insert(&invoice)?;
        debug!(
            "Now tracking invoice to subaddress index {}",
            invoice.index()
        );

        // Return invoice id so the user can build identify their invoice, and make a subscriber for
        // it if desired.
        Ok(invoice.id())
    }

    /// Remove (i.e. stop tracking) invoice, returning the old invoice if it existed.
    ///
    /// # Errors
    ///
    /// Returns an error if there are any underlying issues modifying/retrieving data in the
    /// database.
    pub fn remove_invoice(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, AcceptXmrError> {
        match self.invoices_db.remove(invoice_id)? {
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
                    .insert(invoice_id.sub_index, old.address());

                Ok(Some(old))
            }
            None => Ok(None),
        }
    }

    /// Returns a `Subscriber` for the given invoice ID. If a tracked invoice exists for that
    /// ID, the subscriber can be used to receive updates to for that invoice.
    ///
    /// # Errors
    ///
    /// Returns an error if there is an underlying issue retrieving data from the database.
    pub fn subscribe(&self, invoice_id: InvoiceId) -> Result<Option<Subscriber>, AcceptXmrError> {
        Ok(self.invoices_db.subscribe(invoice_id)?)
    }

    /// Returns a `Subscriber` for all invoices.
    #[must_use]
    pub fn subscribe_all(&self) -> Subscriber {
        self.invoices_db.subscribe_all()
    }

    /// Get current height of daemon using a monero daemon remote procedure call.
    ///
    /// # Errors
    ///
    /// Returns an error if a connection can not be made to the daemon, or if the daemon's response
    /// cannot be parsed.
    pub async fn daemon_height(&self) -> Result<u64, AcceptXmrError> {
        Ok(self.rpc_client.daemon_height().await?)
    }

    /// Get the up-to-date invoice associated with the given [`InvoiceId`], if it exists.
    ///
    /// # Errors
    ///
    /// Returns an error if there are any underlying issues retrieving data in the database.
    pub fn get_invoice(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, AcceptXmrError> {
        Ok(self.invoices_db.get(invoice_id)?)
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
    rpc_timeout: Duration,
    rpc_connection_timeout: Duration,
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
            rpc_timeout: DEFAULT_RPC_TOTAL_TIMEOUT,
            rpc_connection_timeout: DEFAULT_RPC_CONNECTION_TIMEOUT,
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
    /// # Panics
    ///
    /// Panics if the provided URL cannot be parsed.
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
        url.parse::<Uri>().expect("invalid daemon URL");
        self.daemon_url = url.to_string();
        self
    }

    /// Time before an remote procedure call times out. If this amount of time elapses without
    /// receiving a full response from the RPC daemon, the current scan will be aborted and
    /// restarted. Defaults to 10 seconds.
    #[must_use]
    pub fn rpc_timeout(mut self, timeout: Duration) -> PaymentGatewayBuilder {
        self.rpc_timeout = timeout;
        self
    }

    /// Time before a remote procedure call times out while failing to connect. If this amount of
    /// time elapses without managing to connect to the monero daemon, the current scan will be
    /// aborted and restarted. Defaults to 5 seconds.
    #[must_use]
    pub fn rpc_connection_timeout(mut self, timeout: Duration) -> PaymentGatewayBuilder {
        self.rpc_connection_timeout = timeout;
        self
    }

    /// Set the minimum scan interval. New blocks and transactions will be scanned for relevant
    /// outputs at most every `interval`. Defaults to 1 second.
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
    /// Panics if the database cannot be opened at the path specified, or if the internal RPC client
    /// cannot parse the provided URL.
    #[must_use]
    pub fn build(self) -> PaymentGateway {
        let rpc_client = RpcClient::new(
            &self.daemon_url,
            self.rpc_timeout,
            self.rpc_connection_timeout,
        );
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
            block_cache_height: Arc::new(atomic::AtomicU64::new(0)),
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
