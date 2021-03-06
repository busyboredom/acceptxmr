use std::{
    ops::Deref,
    str::FromStr,
    sync::atomic::{self, AtomicU32, AtomicU64},
    sync::{mpsc, Arc, Mutex, PoisonError},
    thread::{self, JoinHandle},
    time::Duration,
};

use hyper::Uri;
use log::{debug, error, info, warn};
use monero::cryptonote::onetime_key::SubKeyChecker;
use tokio::{join, runtime::Runtime, time};

use crate::{
    caching::SubaddressCache,
    invoices_db::InvoicesDb,
    rpc::RpcClient,
    scanner::Scanner,
    subscriber::Subscriber,
    {AcceptXmrError, Invoice, InvoiceId},
};

const DEFAULT_SCAN_INTERVAL: Duration = Duration::from_millis(1000);
const DEFAULT_DAEMON: &str = "http://node.moneroworld.com:18089";
const DEFAULT_DB_PATH: &str = "AcceptXMR_DB";
const DEFAULT_RPC_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_RPC_TOTAL_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_BLOCK_CACHE_SIZE: usize = 10;

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
    cached_daemon_height: Arc<AtomicU64>,
    scanner_handle: Mutex<Option<JoinHandle<Result<(), AcceptXmrError>>>>,
    scanner_command_sender: (
        Mutex<mpsc::Sender<MessageToScanner>>,
        Arc<Mutex<mpsc::Receiver<MessageToScanner>>>,
    ),
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
    pub fn builder(private_view_key: String, primary_address: String) -> PaymentGatewayBuilder {
        PaymentGatewayBuilder::new(private_view_key, primary_address)
    }

    /// Runs the payment gateway. This function spawns a new thread, which periodically scans new
    /// blocks and transactions from the configured daemon and updates pending [`Invoice`](Invoice)s
    /// in the database.
    ///
    /// # Errors
    ///
    /// * Returns an [`AcceptXmrError::InvoiceStorage`] error if there was an underlying issue with
    ///   the database.
    ///
    /// * Returns an [`AcceptXmrError::Rpc`] error if there was an issue getting necessary data from
    ///   the monero daemon while starting.
    ///
    /// * Returns an [`AcceptXmrError::AlreadyRunning`] error if the payment gateway is already
    ///   running.
    ///
    /// * Returns an [`AcceptXmrError::Threading`] error if there was an error creating the scanning
    ///   thread.
    #[allow(clippy::range_plus_one, clippy::missing_panics_doc)]
    pub async fn run(&self) -> Result<(), AcceptXmrError> {
        // Determine if the scanning thread is already running.
        {
            let scanner_handle = self
                .scanner_handle
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(handle) = scanner_handle.as_ref() {
                if !handle.is_finished() {
                    return Err(AcceptXmrError::AlreadyRunning);
                }
            };
        }

        // Gather info needed by the scanner.
        let rpc_client = self.rpc_client.clone();
        let viewpair = self.viewpair;
        let scan_interval = self.scan_interval;
        let highest_minor_index = self.highest_minor_index.clone();
        let block_cache_height = self.block_cache_height.clone();
        let cached_daemon_height = self.cached_daemon_height.clone();
        let pending_invoices = self.invoices_db.clone();
        let command_receiver = self.scanner_command_sender.1.clone();

        // Create scanner.
        debug!("Creating blockchain scanner");
        let mut scanner = Scanner::new(
            rpc_client,
            pending_invoices,
            DEFAULT_BLOCK_CACHE_SIZE,
            block_cache_height,
            cached_daemon_height,
        )
        .await?;

        // Spawn the scanning thread.
        info!("Starting blockchain scanner");
        *self.scanner_handle.lock().unwrap_or_else(PoisonError::into_inner) = Some(thread::Builder::new()
            .name("Scanning Thread".to_string())
            .spawn(move || -> Result<(), AcceptXmrError> {
                // The thread needs a tokio runtime to process async functions.
                let tokio_runtime = Runtime::new()?;
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
                        // If we're received the stop signal, stop.
                        match command_receiver.lock().unwrap_or_else(PoisonError::into_inner).try_recv() {
                            Ok(MessageToScanner::Stop) => {
                                info!("Scanner received stop signal. Stopping gracefully");
                                break;
                            }
                            Err(mpsc::TryRecvError::Empty) => {
                            }
                            Err(mpsc::TryRecvError::Disconnected) => {
                                error!("Scanner lost connection to payment gateway. Stopping gracefully.");
                                break;
                            }
                        }
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
                        if let (_, Err(e)) = join!(blockscan_interval.tick(), scanner.scan(&sub_key_checker)) {
                            error!("Payment gateway encountered an error while scanning for payments: {}", e);
                        };
                    }
                });
                Ok(())
            })?);
        debug!("Scanner started successfully");
        Ok(())
    }

    /// Returns the enum [`PaymentGatewayStatus`] describing whether the payment gateway is running,
    /// not running, or has experienced an error.
    #[must_use]
    pub fn status(&self) -> PaymentGatewayStatus {
        let scanner_handle = self
            .scanner_handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match scanner_handle.as_ref() {
            None => PaymentGatewayStatus::NotRunning,
            Some(handle) if handle.is_finished() => {
                let owned_handle = self
                    .scanner_handle
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                match owned_handle.map(std::thread::JoinHandle::join) {
                    None | Some(Ok(Ok(_))) => PaymentGatewayStatus::NotRunning,
                    Some(Ok(Err(e))) => {
                        PaymentGatewayStatus::Error(AcceptXmrError::ScanningThread(Box::new(e)))
                    }
                    Some(Err(_)) => {
                        PaymentGatewayStatus::Error(AcceptXmrError::ScanningThreadPanic)
                    }
                }
            }
            Some(_) => PaymentGatewayStatus::Running,
        }
    }

    /// Stops the payment gateway, blocking until complete. If the payment gateway is not running,
    /// this method does nothing.
    ///
    /// # Errors
    ///
    /// * Returns an [`AcceptXmrError::StopSignal`] error if the payment gateway could not be stopped.
    ///
    /// * Returns an [`AcceptXmrError::ScanningThread`] error if the scanning thread exited with an error.
    ///
    /// * Returns an [`AcceptXmrError::ScanningThreadPanic`] error if the scanning thread exited with a panic.
    pub fn stop(&self) -> Result<(), AcceptXmrError> {
        match self
            .scanner_handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            None => Ok(()),
            Some(thread) if thread.is_finished() => match thread.join() {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(e)) => Err(AcceptXmrError::ScanningThread(Box::new(e))),
                Err(_) => Err(AcceptXmrError::ScanningThreadPanic),
            },
            Some(thread) => {
                self.scanner_command_sender
                    .0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .send(MessageToScanner::Stop)
                    .map_err(|e| AcceptXmrError::StopSignal(e.to_string()))?;
                match thread.join() {
                    Ok(Ok(_)) => Ok(()),
                    Ok(Err(e)) => Err(AcceptXmrError::ScanningThread(Box::new(e))),
                    Err(_) => Err(AcceptXmrError::ScanningThreadPanic),
                }
            }
        }
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
        description: String,
    ) -> Result<InvoiceId, AcceptXmrError> {
        let amount = piconeros;

        // Get subaddress in base58, and subaddress index.
        let (sub_index, subaddress) = self
            .subaddresses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove_random();

        let creation_height = self.cached_daemon_height.load(atomic::Ordering::Relaxed);

        // Create invoice object.
        let invoice = Invoice::new(
            subaddress,
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
                    .insert(invoice_id.sub_index, old.address().to_string());

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

    /// Get current height of block cache.
    #[must_use]
    #[doc(hidden)]
    pub fn cache_height(&self) -> u64 {
        use std::sync::atomic::Ordering;
        self.block_cache_height.load(Ordering::Relaxed)
    }

    /// Get the up-to-date invoice associated with the given [`InvoiceId`], if it exists.
    ///
    /// # Errors
    ///
    /// Returns an error if there are any underlying issues retrieving data from the database.
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
/// let primary_address = "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";
///
/// // Create a payment gateway with an extra fast scan rate and a custom monero daemon URL.
/// let payment_gateway = PaymentGatewayBuilder::new(private_view_key.to_string(), primary_address.to_string())
///     .scan_interval(Duration::from_millis(100)) // Scan for invoice updates every 100 ms.
///     .daemon_url("http://example.com:18081") // Set custom monero daemon URL.
/// #   .db_path(temp_dir.path().to_str().expect("Failed to get temporary directory path").to_string())
///     .build();
/// ```
pub struct PaymentGatewayBuilder {
    daemon_url: String,
    rpc_timeout: Duration,
    rpc_connection_timeout: Duration,
    private_view_key: String,
    primary_address: String,
    scan_interval: Duration,
    db_path: String,
    seed: Option<u64>,
}

impl PaymentGatewayBuilder {
    /// Create a new payment gateway builder.
    #[must_use]
    pub fn new(private_view_key: String, primary_address: String) -> PaymentGatewayBuilder {
        PaymentGatewayBuilder {
            daemon_url: DEFAULT_DAEMON.to_string(),
            rpc_timeout: DEFAULT_RPC_TOTAL_TIMEOUT,
            rpc_connection_timeout: DEFAULT_RPC_CONNECTION_TIMEOUT,
            private_view_key,
            primary_address,
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
    /// let primary_address = "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";
    ///
    /// // Create a payment gateway with a custom monero daemon URL.
    /// let payment_gateway = PaymentGatewayBuilder::new(private_view_key.to_string(), primary_address.to_string())
    ///     .daemon_url("http://example.com:18081") // Set custom monero daemon URL.
    ///     .build()?;
    ///
    /// // The payment gateway will now use the daemon specified.
    /// payment_gateway.run().await?;
    /// #   Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn daemon_url(mut self, url: &str) -> PaymentGatewayBuilder {
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
    pub fn db_path(mut self, path: String) -> PaymentGatewayBuilder {
        self.db_path = path;
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
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened at the path specified, if the internal RPC
    /// client cannot parse the provided URL, or if the primary address or private view key cannot
    /// be parsed.
    pub fn build(self) -> Result<PaymentGateway, AcceptXmrError> {
        let rpc_client = RpcClient::new(
            self.daemon_url
                .parse::<Uri>()
                .map_err(|e| AcceptXmrError::Parse {
                    datatype: "Uri",
                    input: self.daemon_url,
                    error: e.to_string(),
                })?,
            self.rpc_timeout,
            self.rpc_connection_timeout,
        );
        let invoices_db = InvoicesDb::new(&self.db_path)?;
        info!("Opened database in \"{}/\"", self.db_path);

        let viewpair = monero::ViewPair {
            view: monero::PrivateKey::from_str(&self.private_view_key).map_err(|e| {
                AcceptXmrError::Parse {
                    datatype: "PrivateKey",
                    input: self.private_view_key.to_string(),
                    error: e.to_string(),
                }
            })?,
            spend: monero::Address::from_str(&self.primary_address)
                .map_err(|e| AcceptXmrError::Parse {
                    datatype: "Address",
                    input: self.primary_address.to_string(),
                    error: e.to_string(),
                })?
                .public_spend,
        };

        let highest_minor_index = Arc::new(AtomicU32::new(0));
        let subaddresses = SubaddressCache::init(
            &invoices_db,
            viewpair,
            highest_minor_index.clone(),
            self.seed,
        )?;
        debug!("Generated {} initial subaddresses", subaddresses.len());

        let (scanner_tx, scanner_rx) = mpsc::channel();
        let scanner_command_sender = (Mutex::new(scanner_tx), Arc::new(Mutex::new(scanner_rx)));

        Ok(PaymentGateway(Arc::new(PaymentGatewayInner {
            rpc_client,
            viewpair,
            scan_interval: self.scan_interval,
            invoices_db,
            subaddresses: Mutex::new(subaddresses),
            highest_minor_index,
            block_cache_height: Arc::new(atomic::AtomicU64::new(0)),
            cached_daemon_height: Arc::new(atomic::AtomicU64::new(0)),
            scanner_handle: Mutex::new(None),
            scanner_command_sender,
        })))
    }
}

/// Enumeration of possible payment gateway states.
#[derive(Debug)]
pub enum PaymentGatewayStatus {
    /// The payment gateway is scanning for incoming payments.
    Running,
    /// The payment gateway is not scanning for incoming payments.
    NotRunning,
    /// The payment gateway encountered an error while scanning for incoming payments, and had to
    /// stop.
    Error(AcceptXmrError),
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub(crate) enum MessageToScanner {
    Stop,
}

#[cfg(test)]
#[allow(clippy::expect_used)]
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
    const PRIMARY_ADDRESS: &str =
        "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";

    #[test]
    fn daemon_url() {
        // Setup.
        init_logger();
        let temp_dir = new_temp_dir();

        let payment_gateway =
            PaymentGatewayBuilder::new(PRIVATE_VIEW_KEY.to_string(), PRIMARY_ADDRESS.to_string())
                .db_path(
                    temp_dir
                        .path()
                        .to_str()
                        .expect("failed to get temporary directory path")
                        .to_string(),
                )
                .daemon_url("http://example.com:18081")
                .build()
                .expect("failed to build payment gateway");

        assert_eq!(
            payment_gateway.rpc_client.url(),
            "http://example.com:18081/"
        );
    }
}
