#![warn(clippy::pedantic)]

mod block_cache;
mod payments_db;
mod rcp;
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
pub use payments_db::PaymentStorageError;
use payments_db::PaymentsDb;
use scanner::Scanner;
use subaddress_cache::SubaddressCache;
pub use subscriber::Subscriber;
use txpool_cache::TxpoolCache;
pub use util::AcceptXmrError;

const DEFAULT_SCAN_INTERVAL: Duration = Duration::from_millis(1000);

#[derive(Clone)]
pub struct PaymentGateway(pub(crate) Arc<PaymentGatewayInner>);

#[doc(hidden)]
pub struct PaymentGatewayInner {
    daemon_url: String,
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
    #[must_use]
    pub fn builder() -> PaymentGatewayBuilder {
        PaymentGatewayBuilder::default()
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn run(&self, cache_size: u64) {
        // Gather info needed by the scanner.
        let url = self.0.daemon_url.clone();
        let viewpair = monero::ViewPair {
            view: self.0.viewpair.view,
            spend: self.0.viewpair.spend,
        };
        let scan_interval = self.scan_interval;
        let atomic_height = self.height.clone();
        let pending_payments = self.payments_db.clone();

        // Spawn the scanning thread.
        info!("Starting blockchain scanner");
        thread::Builder::new()
            .name("Scanning Thread".to_string())
            .spawn(move || {
                // The thread needs a tokio runtime to process async functions.
                let tokio_runtime = Runtime::new().unwrap();
                tokio_runtime.block_on(async move {
                    // Create scanner.
                    let mut scanner =
                        Scanner::new(url, viewpair, pending_payments, cache_size, atomic_height)
                            .await;

                    // Scan for transactions once every scan_interval.
                    let mut blockscan_interval = time::interval(scan_interval);
                    loop {
                        join!(blockscan_interval.tick(), scanner.scan());
                    }
                });
            })
            .expect("Error spawning scanning thread");
    }

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
        Ok(self.watch_payment(sub_index))
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

    #[must_use]
    pub fn watch_payment(&self, sub_index: SubIndex) -> Subscriber {
        self.payments_db.watch_payment(sub_index)
    }

    /// Get current height of daemon using a monero daemon RPC call.
    ///
    /// # Errors
    ///
    /// Returns and error if a connection can not be made to the daemon, or if the daemon's response
    /// cannot be parsed.
    pub async fn daemon_height(&self) -> Result<u64, AcceptXmrError> {
        Ok(rcp::daemon_height(&self.daemon_url).await?)
    }
}

#[derive(Default)]
pub struct PaymentGatewayBuilder {
    daemon_url: String,
    private_viewkey: Option<monero::PrivateKey>,
    public_spendkey: Option<monero::PublicKey>,
    scan_interval: Option<Duration>,
    db_path: Option<String>,
}

impl PaymentGatewayBuilder {
    #[must_use]
    pub fn new() -> PaymentGatewayBuilder {
        PaymentGatewayBuilder::default()
    }

    #[must_use]
    pub fn daemon_url(mut self, url: &str) -> PaymentGatewayBuilder {
        reqwest::Url::parse(url).expect("invalid daemon URL");
        self.daemon_url = url.to_string();
        self
    }

    #[must_use]
    pub fn private_viewkey(mut self, private_viewkey: &str) -> PaymentGatewayBuilder {
        self.private_viewkey =
            Some(monero::PrivateKey::from_str(private_viewkey).expect("invalid private viewkey"));
        self
    }

    #[must_use]
    pub fn public_spendkey(mut self, public_spendkey: &str) -> PaymentGatewayBuilder {
        self.public_spendkey =
            Some(monero::PublicKey::from_str(public_spendkey).expect("invalid public spendkey"));
        self
    }

    #[must_use]
    pub fn scan_interval(mut self, interval: Duration) -> PaymentGatewayBuilder {
        self.scan_interval = Some(interval);
        self
    }

    #[must_use]
    pub fn db_path(mut self, path: &str) -> PaymentGatewayBuilder {
        self.db_path = Some(path.to_string());
        self
    }

    #[must_use]
    pub fn build(self) -> PaymentGateway {
        let private_viewkey = self
            .private_viewkey
            .expect("private viewkey must be defined");
        let public_spendkey = self
            .public_spendkey
            .expect("public spendkey must be defined");
        let scan_interval = self.scan_interval.unwrap_or(DEFAULT_SCAN_INTERVAL);
        let db_path = self.db_path.unwrap_or_else(|| "AcceptXMR_DB".to_string());
        let payments_db =
            PaymentsDb::new(&db_path).expect("failed to open pending payments database tree");
        info!("Opened database in \"{}/\"", db_path);
        let viewpair = monero::ViewPair {
            view: private_viewkey,
            spend: public_spendkey,
        };
        let subaddresses = SubaddressCache::init(&payments_db, viewpair);
        debug!("Generated {} initial subaddresses", subaddresses.len());

        PaymentGateway(Arc::new(PaymentGatewayInner {
            daemon_url: self.daemon_url,
            viewpair,
            scan_interval,
            payments_db,
            subaddresses: Mutex::new(subaddresses),
            height: Arc::new(atomic::AtomicU64::new(0)),
        }))
    }
}

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

    #[must_use]
    pub fn is_confirmed(&self) -> bool {
        self.confirmations().map_or(false, |confirmations| {
            confirmations >= self.confirmations_required
        })
    }

    #[must_use]
    pub fn is_expired(&self) -> bool {
        // At or passed the expiration block, AND not paid in full.
        self.current_height >= self.expiration_at && self.paid_at.is_none()
    }

    #[must_use]
    pub fn address(&self) -> String {
        self.address.clone()
    }

    #[must_use]
    pub fn index(&self) -> SubIndex {
        self.index
    }

    #[must_use]
    pub fn started_at(&self) -> u64 {
        self.started_at
    }

    #[must_use]
    pub fn amount_requested(&self) -> u64 {
        self.amount_requested
    }

    #[must_use]
    pub fn amount_paid(&self) -> u64 {
        self.amount_paid
    }

    #[must_use]
    pub fn confirmations_required(&self) -> u64 {
        self.confirmations_required
    }

    #[must_use]
    pub fn confirmations(&self) -> Option<u64> {
        self.paid_at
            .map(|paid_at| self.current_height.saturating_sub(paid_at) + 1)
    }

    #[must_use]
    pub fn current_height(&self) -> u64 {
        self.current_height
    }

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

#[derive(Debug, Copy, Clone, Hash, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubIndex {
    pub major: u32,
    pub minor: u32,
}

impl SubIndex {
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Copy)]
pub struct Transfer {
    pub amount: u64,
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
