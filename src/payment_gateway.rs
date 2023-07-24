use std::{
    fmt::Debug,
    ops::Deref,
    str::FromStr,
    sync::{
        atomic::{self, AtomicU32, AtomicU64},
        mpsc::{channel, Receiver, Sender, TryRecvError},
        Arc, Mutex, PoisonError, RwLock,
    },
    thread,
    time::Duration,
};

use hyper::Uri;
use log::{debug, error, info, warn};
use monero::cryptonote::onetime_key::SubKeyChecker;
use tokio::{join, runtime::Runtime, time};

use crate::{
    caching::SubaddressCache,
    pubsub::{Publisher, Subscriber},
    rpc::RpcClient,
    scanner::{Scanner, ScannerHandle},
    storage::{InvoiceStorage, Storage},
    AcceptXmrError, Invoice, InvoiceId,
};

const DEFAULT_SCAN_INTERVAL: Duration = Duration::from_millis(1000);
const DEFAULT_DAEMON: &str = "http://node.moneroworld.com:18089";
/// Timeout for RPC connection formation.
const DEFAULT_RPC_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
/// Timeout for total call completion.
const DEFAULT_RPC_TOTAL_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_BLOCK_CACHE_SIZE: usize = 10;

/// The `PaymentGateway` allows you to track new [`Invoice`](Invoice)s, remove
/// old `Invoice`s from tracking, and subscribe to `Invoice`s that are already
/// pending.
pub struct PaymentGateway<S: Storage>(pub(crate) Arc<PaymentGatewayInner<S>>);

#[doc(hidden)]
pub struct PaymentGatewayInner<S: Storage> {
    rpc_client: RpcClient,
    viewpair: monero::ViewPair,
    scan_interval: Duration,
    store: Arc<RwLock<S>>,
    subaddresses: Mutex<SubaddressCache>,
    major_index: u32,
    highest_minor_index: Arc<AtomicU32>,
    initial_height: Option<u64>,
    block_cache_height: Arc<AtomicU64>,
    cached_daemon_height: Arc<AtomicU64>,
    scanner_handle: Mutex<Option<ScannerHandle>>,
    /// Send commands to the scanning thread.
    scanner_command_sender: (
        Mutex<Sender<MessageToScanner>>,
        Arc<Mutex<Receiver<MessageToScanner>>>,
    ),
    publisher: Arc<Publisher>,
}

impl<S: Storage> Clone for PaymentGateway<S> {
    fn clone(&self) -> Self {
        PaymentGateway(self.0.clone())
    }
}

impl<S: Storage> Deref for PaymentGateway<S> {
    type Target = PaymentGatewayInner<S>;

    fn deref(&self) -> &PaymentGatewayInner<S> {
        &self.0
    }
}

impl<S: Storage + 'static> PaymentGateway<S> {
    /// Returns a builder used to create a new payment gateway.
    #[must_use]
    pub fn builder(
        private_view_key: String,
        primary_address: String,
        store: S,
    ) -> PaymentGatewayBuilder<S> {
        PaymentGatewayBuilder::new(private_view_key, primary_address, store)
    }

    /// Runs the payment gateway. This function spawns a new thread, which
    /// periodically scans new blocks and transactions from the configured
    /// daemon and updates pending [`Invoice`](Invoice)s in the database.
    ///
    /// # Errors
    ///
    /// * Returns an [`AcceptXmrError::Storage`] error if there was an
    ///   underlying issue with the database.
    ///
    /// * Returns an [`AcceptXmrError::Rpc`] error if there was an issue getting
    ///   necessary data from the monero daemon while starting.
    ///
    /// * Returns an [`AcceptXmrError::AlreadyRunning`] error if the payment
    ///   gateway is already running.
    ///
    /// * Returns an [`AcceptXmrError::Threading`] error if there was an error
    ///   creating the scanning thread.
    #[allow(clippy::range_plus_one)]
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
        let major_index = self.major_index;
        let highest_minor_index = self.highest_minor_index.clone();
        let block_cache_height = self.block_cache_height.clone();
        let cached_daemon_height = self.cached_daemon_height.clone();
        let initial_height = self.initial_height;
        let publisher = self.publisher.clone();
        let store = self.store.clone();
        let command_receiver = self.scanner_command_sender.1.clone();

        // Create scanner.
        debug!("Creating blockchain scanner");
        let mut scanner: Scanner<S> = Scanner::new(
            rpc_client,
            store,
            DEFAULT_BLOCK_CACHE_SIZE,
            block_cache_height,
            cached_daemon_height,
            initial_height,
            publisher,
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
                            Err(TryRecvError::Empty) => {
                            }
                            Err(TryRecvError::Disconnected) => {
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
                                major_index..major_index.saturating_add(1),
                                0..highest_minor_index.load(atomic::Ordering::Relaxed).saturating_add(1),
                            );
                        }
                        // Scan!
                        if let (_, Err(e)) = join!(blockscan_interval.tick(), scanner.scan(&sub_key_checker)) {
                            error!("Payment gateway encountered an error while scanning for payments: {}", e);
                        };
                    }
                });
                Ok(())
            })?.into());
        debug!("Scanner started successfully");
        Ok(())
    }

    /// Returns the enum [`PaymentGatewayStatus`] describing whether the payment
    /// gateway is running, not running, or has experienced an error.
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
                match owned_handle.map(ScannerHandle::join) {
                    None | Some(Ok(Ok(_))) => PaymentGatewayStatus::NotRunning,
                    Some(Ok(Err(e))) => PaymentGatewayStatus::Error(e),
                    Some(Err(_)) => {
                        PaymentGatewayStatus::Error(AcceptXmrError::ScanningThreadPanic)
                    }
                }
            }
            Some(_) => PaymentGatewayStatus::Running,
        }
    }

    /// Stops the payment gateway, blocking until complete. If the payment
    /// gateway is not running, this method does nothing.
    ///
    /// # Errors
    ///
    /// * Returns an [`AcceptXmrError::StopSignal`] error if the payment gateway
    ///   could not be stopped.
    ///
    /// * Returns an [`AcceptXmrError::ScanningThreadPanic`] error if the
    ///   scanning thread exited with a panic.
    ///
    /// * If the scanning thread exited with an error, returns the error
    ///   encountered.
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
                Ok(Err(e)) => Err(e),
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
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(AcceptXmrError::ScanningThreadPanic),
                }
            }
        }
    }

    /// Adds a new [`Invoice`] to the payment gateway for tracking, and returns
    /// the ID of the new invoice. Use a [`Subscriber`] to receive updates
    /// on the new invoice invoice as they occur.
    ///
    /// # Errors
    ///
    /// Returns an error if there are any underlying issues modifying data in
    /// the database.
    pub fn new_invoice(
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
        InvoiceStorage::insert(
            &mut *self.store.write().unwrap_or_else(PoisonError::into_inner),
            invoice.clone(),
        )
        .map_err(|e| AcceptXmrError::Storage(Box::new(e)))?;
        debug!(
            "Now tracking invoice to subaddress index {}",
            invoice.index()
        );

        self.publisher.insert_invoice(invoice.id());

        // Return invoice id so the user can build identify their invoice, and make a
        // subscriber for it if desired.
        Ok(invoice.id())
    }

    /// Remove (i.e. stop tracking) invoice, returning the old invoice if it
    /// existed.
    ///
    /// # Errors
    ///
    /// Returns an error if there are any underlying issues modifying/retrieving
    /// data in the database.
    pub fn remove_invoice(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, AcceptXmrError> {
        match self
            .store
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(invoice_id)
            .map_err(|e| AcceptXmrError::Storage(Box::new(e)))?
        {
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

                // Kill any related subscriptions.
                self.publisher.remove_invoice(invoice_id);

                Ok(Some(old))
            }
            None => Ok(None),
        }
    }

    /// Returns a `Subscriber` for the given invoice ID. If a tracked invoice
    /// exists for that ID, the subscriber can be used to receive updates to
    /// for that invoice.
    #[must_use]
    pub fn subscribe(&self, invoice_id: InvoiceId) -> Option<Subscriber> {
        self.publisher.subscribe(invoice_id)
    }

    /// Returns a `Subscriber` for all invoices.
    #[must_use]
    pub fn subscribe_all(&self) -> Subscriber {
        self.publisher.subscribe_all()
    }

    /// Get current height of daemon using a monero daemon remote procedure
    /// call.
    ///
    /// # Errors
    ///
    /// Returns an error if a connection can not be made to the daemon, or if
    /// the daemon's response cannot be parsed.
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

    /// Get the up-to-date invoice associated with the given [`InvoiceId`], if
    /// it exists.
    ///
    /// # Errors
    ///
    /// Returns an error if there are any underlying issues retrieving data from
    /// the database.
    pub fn get_invoice(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, AcceptXmrError> {
        InvoiceStorage::get(
            &*self.store.read().unwrap_or_else(PoisonError::into_inner),
            invoice_id,
        )
        .map_err(|e| AcceptXmrError::Storage(Box::new(e)))
    }

    /// Returns URL of configured daemon.
    #[must_use]
    pub fn daemon_url(&self) -> String {
        self.rpc_client.url()
    }
}

/// A builder for the payment gateway. Used to configure your desired monero
/// daemon, scan interval, view key, etc.
///
/// # Examples
///
/// ```
/// use acceptxmr::{PaymentGatewayBuilder, storage::stores::InMemory};
/// use std::time::Duration;
///
/// let private_view_key =
///     "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
/// let primary_address =
///     "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";
///
/// let store = InMemory::new();
///
/// // Create a payment gateway with an extra fast scan rate and a custom monero daemon URL.
/// let payment_gateway = PaymentGatewayBuilder::new(
///     private_view_key.to_string(),
///     primary_address.to_string(),
///     store
/// )
/// .scan_interval(Duration::from_millis(100)) // Scan for updates every 100 ms.
/// .daemon_url("http://example.com:18081".to_string()) // Set custom monero daemon URL.
/// .build();
/// ```
pub struct PaymentGatewayBuilder<S> {
    daemon_url: String,
    daemon_username: Option<String>,
    daemon_password: Option<String>,
    rpc_timeout: Duration,
    rpc_connection_timeout: Duration,
    private_view_key: String,
    primary_address: String,
    scan_interval: Duration,
    store: S,
    major_index: u32,
    initial_height: Option<u64>,
    seed: Option<u64>,
}

impl<S: Storage + 'static> PaymentGatewayBuilder<S> {
    /// Create a new payment gateway builder.
    #[must_use]
    pub fn new(
        private_view_key: String,
        primary_address: String,
        store: S,
    ) -> PaymentGatewayBuilder<S> {
        PaymentGatewayBuilder {
            daemon_url: DEFAULT_DAEMON.to_string(),
            daemon_username: None,
            daemon_password: None,
            rpc_timeout: DEFAULT_RPC_TOTAL_TIMEOUT,
            rpc_connection_timeout: DEFAULT_RPC_CONNECTION_TIMEOUT,
            private_view_key,
            primary_address,
            scan_interval: DEFAULT_SCAN_INTERVAL,
            store,
            major_index: 0,
            initial_height: None,
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
    /// use acceptxmr::{PaymentGatewayBuilder, storage::stores::InMemory};
    ///
    /// let private_view_key =
    ///     "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
    /// let primary_address =
    ///     "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";
    ///
    /// // Pick a storage layer. We'll store data in-memory here for simplicity.
    /// let store = InMemory::new();
    ///
    /// // Create a payment gateway with a custom monero daemon URL.
    /// let payment_gateway = PaymentGatewayBuilder::new(
    ///     private_view_key.to_string(),
    ///     primary_address.to_string(),
    ///     store
    /// )
    /// .daemon_url("http://example.com:18081".to_string()) // Set custom monero daemon URL.
    /// .build()?;
    ///
    /// // The payment gateway will now use the daemon specified.
    /// payment_gateway.run().await?;
    /// #   Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn daemon_url(mut self, url: String) -> PaymentGatewayBuilder<S> {
        self.daemon_url = url;
        self
    }

    /// If your preferred daemon requires a password, configure it here.
    #[must_use]
    pub fn daemon_login(mut self, username: String, password: String) -> PaymentGatewayBuilder<S> {
        self.daemon_username = Some(username);
        self.daemon_password = Some(password);
        self
    }

    /// Time before an remote procedure call times out. If this amount of time
    /// elapses without receiving a full response from the RPC daemon, the
    /// current scan will be aborted and restarted. Defaults to 10 seconds.
    #[must_use]
    pub fn rpc_timeout(mut self, timeout: Duration) -> PaymentGatewayBuilder<S> {
        self.rpc_timeout = timeout;
        self
    }

    /// Time before a remote procedure call times out while failing to connect.
    /// If this amount of time elapses without managing to connect to the
    /// monero daemon, the current scan will be aborted and restarted.
    /// Defaults to 5 seconds.
    #[must_use]
    pub fn rpc_connection_timeout(mut self, timeout: Duration) -> PaymentGatewayBuilder<S> {
        self.rpc_connection_timeout = timeout;
        self
    }

    /// Set the minimum scan interval. New blocks and transactions will be
    /// scanned for relevant outputs at most every `interval`. Defaults to 1
    /// second.
    #[must_use]
    pub fn scan_interval(mut self, interval: Duration) -> PaymentGatewayBuilder<S> {
        self.scan_interval = interval;
        self
    }

    /// Seed for random number generator. Use only for reproducible testing. Do
    /// not set in a production environment.
    #[must_use]
    pub fn seed(mut self, seed: u64) -> PaymentGatewayBuilder<S> {
        warn!("Seed set to {}. Some operations intended to be random (like the order in which subaddresses are used) will be predictable.", seed);
        self.seed = Some(seed);
        self
    }

    /// Set the account index (i.e. subaddress major index) the payment gateway
    /// should use. Defaults to account index 0.
    #[must_use]
    pub fn account_index(mut self, index: u32) -> PaymentGatewayBuilder<S> {
        self.major_index = index;
        self
    }

    /// Set the initial height that the payment gateway should start scanning
    /// from. For best protection against the burning bug, this should be set to
    /// your wallet's restore height.
    ///
    /// This method only affects new payment gateways. If a payment gateway has
    /// already scanned blocks higher than the specified initial height, then it
    /// will continue scanning from the height where it left off.
    ///
    /// Defaults to the current blockchain tip. Setting an initial height
    /// greater than the blockchain tip will do nothing.
    #[must_use]
    pub fn initial_height(mut self, height: u64) -> PaymentGatewayBuilder<S> {
        self.initial_height = Some(height);
        self
    }

    /// Build the payment gateway.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened at the path specified,
    /// if the internal RPC client cannot parse the provided URL, or if the
    /// primary address or private view key cannot be parsed.
    pub fn build(self) -> Result<PaymentGateway<S>, AcceptXmrError> {
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
            self.daemon_username,
            self.daemon_password,
            self.seed,
        );

        let store = Arc::new(RwLock::new(self.store));

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
            &store,
            viewpair,
            self.major_index,
            highest_minor_index.clone(),
            self.seed,
        )
        .map_err(|e| AcceptXmrError::Storage(Box::new(e)))?;
        debug!("Generated {} initial subaddresses", subaddresses.len());

        let (scanner_cmd_tx, scanner_cmd_rx) = channel();
        let scanner_command_sender = (
            Mutex::new(scanner_cmd_tx),
            Arc::new(Mutex::new(scanner_cmd_rx)),
        );

        Ok(PaymentGateway(Arc::new(PaymentGatewayInner {
            rpc_client,
            viewpair,
            scan_interval: self.scan_interval,
            store,
            subaddresses: Mutex::new(subaddresses),
            major_index: self.major_index,
            highest_minor_index,
            initial_height: self.initial_height,
            block_cache_height: Arc::new(atomic::AtomicU64::new(0)),
            cached_daemon_height: Arc::new(atomic::AtomicU64::new(0)),
            scanner_handle: Mutex::new(None),
            scanner_command_sender,
            publisher: Arc::new(Publisher::new()),
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
    /// The payment gateway encountered an error while scanning for incoming
    /// payments, and had to stop.
    Error(AcceptXmrError),
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub(crate) enum MessageToScanner {
    Stop,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use log::LevelFilter;

    use crate::{storage::stores::InMemory, PaymentGateway, PaymentGatewayBuilder};

    fn init_logger() {
        env_logger::builder()
            .filter_level(LevelFilter::Warn)
            .filter_module("acceptxmr", log::LevelFilter::Debug)
            .is_test(true)
            .try_init()
            .ok();
    }

    const PRIVATE_VIEW_KEY: &str =
        "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
    const PRIMARY_ADDRESS: &str =
        "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";

    #[test]
    fn daemon_url() {
        // Setup.
        init_logger();
        let store = InMemory::new();

        let payment_gateway: PaymentGateway<InMemory> = PaymentGatewayBuilder::<InMemory>::new(
            PRIVATE_VIEW_KEY.to_string(),
            PRIMARY_ADDRESS.to_string(),
            store,
        )
        .daemon_url("http://example.com:18081".to_string())
        .build()
        .unwrap();

        assert_eq!(
            payment_gateway.rpc_client.url(),
            "http://example.com:18081/"
        );
    }
}
