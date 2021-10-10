mod block_cache;
mod error;
mod payments_db;
mod scanner;
mod subscriber;
mod util;

use std::cmp;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::{atomic, Arc};
use std::{fmt, thread, u64};

use log::{debug, info, warn};
use monero::cryptonote::subaddress;
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;
use tokio::{join, time};

use block_cache::BlockCache;
pub use error::AcceptXMRError;
pub use payments_db::PaymentStorageError;
use payments_db::PaymentsDb;
use scanner::Scanner;
pub use subscriber::Subscriber;

#[derive(Clone)]
pub struct PaymentGateway(pub(crate) Arc<PaymentGatewayInner>);

#[doc(hidden)]
pub struct PaymentGatewayInner {
    daemon_url: String,
    viewpair: monero::ViewPair,
    scan_rate: u64,
    payments_db: PaymentsDb,
    height: Arc<atomic::AtomicU64>,
}

impl Deref for PaymentGateway {
    type Target = PaymentGatewayInner;

    fn deref(&self) -> &PaymentGatewayInner {
        &self.0
    }
}

impl PaymentGateway {
    pub fn builder() -> PaymentGatewayBuilder {
        PaymentGatewayBuilder::default()
    }

    pub fn run(&self, cache_size: u64) {
        // Gather info needed by the scanner.
        let url = self.0.daemon_url.to_owned();
        let viewpair = monero::ViewPair {
            view: self.0.viewpair.view,
            spend: self.0.viewpair.spend,
        };
        let scan_rate = self.scan_rate;
        let atomic_height = self.height.clone();
        let pending_payments = self.payments_db.clone();

        // Spawn the scanning thread.
        info!("Starting blockchain scanner now");
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

                    // Scan for transactions once every scan_rate.
                    let mut blockscan_interval =
                        time::interval(time::Duration::from_millis(scan_rate));
                    loop {
                        join!(blockscan_interval.tick(), scanner.scan());
                    }
                })
            })
            .expect("Error spawning scanning thread");
    }

    /// Panics if xmr is more than u64::MAX.
    pub async fn new_payment(
        &mut self,
        xmr: f64,
        confirmations_required: u64,
        expiration_in: u64,
    ) -> Result<Subscriber, AcceptXMRError> {
        // Convert xmr to picos.
        let amount = monero::Amount::from_xmr(xmr)
            .expect("amount due must be less than u64::MAX")
            .as_pico();

        // Get subaddress in base58, and subaddress index.
        let sub_index = SubIndex::new(0, 1);
        let subaddress = format!(
            "{}",
            subaddress::get_subaddress(&self.viewpair, sub_index.into(), None)
        );

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
        // TODO: Don't return before a flush happens.
        Ok(self.watch_payment(&sub_index))
    }

    pub fn remove_payment(&self, sub_index: &SubIndex) -> Result<Option<Payment>, AcceptXMRError> {
        match self.payments_db.remove(sub_index)? {
            Some(old) => {
                if !(old.is_expired() || old.is_confirmed() && old.started_at < old.current_height)
                {
                    warn!("Removed a payment which was neither expired, nor fully confirmed and a block or more old. Was this intentional?");
                }
                Ok(self.payments_db.remove(sub_index)?)
            }
            None => Ok(None),
        }
    }

    pub fn watch_payment(&self, sub_index: &SubIndex) -> Subscriber {
        self.payments_db.watch_payment(sub_index)
    }

    pub async fn get_daemon_height(&self) -> Result<u64, AcceptXMRError> {
        util::get_daemon_height(&self.daemon_url).await
    }
}

#[derive(Default)]
pub struct PaymentGatewayBuilder {
    daemon_url: String,
    private_viewkey: Option<monero::PrivateKey>,
    public_spendkey: Option<monero::PublicKey>,
    scan_rate: Option<u64>,
    db_path: Option<String>,
}

impl PaymentGatewayBuilder {
    pub fn new() -> PaymentGatewayBuilder {
        PaymentGatewayBuilder::default()
    }

    pub fn daemon_url(mut self, url: &str) -> PaymentGatewayBuilder {
        reqwest::Url::parse(url).expect("invalid daemon URL");
        self.daemon_url = url.to_string();
        self
    }

    pub fn private_viewkey(mut self, private_viewkey: &str) -> PaymentGatewayBuilder {
        self.private_viewkey =
            Some(monero::PrivateKey::from_str(private_viewkey).expect("invalid private viewkey"));
        self
    }

    pub fn public_spendkey(mut self, public_spendkey: &str) -> PaymentGatewayBuilder {
        self.public_spendkey =
            Some(monero::PublicKey::from_str(public_spendkey).expect("invalid public spendkey"));
        self
    }

    pub fn scan_rate(mut self, milliseconds: u64) -> PaymentGatewayBuilder {
        self.scan_rate = Some(milliseconds);
        self
    }

    pub fn db_path(mut self, path: &str) -> PaymentGatewayBuilder {
        self.db_path = Some(path.to_string());
        self
    }

    pub fn build(self) -> PaymentGateway {
        let private_viewkey = self
            .private_viewkey
            .expect("private viewkey must be defined");
        let public_spendkey = self
            .public_spendkey
            .expect("public spendkey must be defined");
        let scan_rate = self.scan_rate.unwrap_or(1000);
        let db_path = self.db_path.unwrap_or_else(|| "AcceptXMR_DB".to_string());
        let payments_db =
            PaymentsDb::new(&db_path).expect("failed to open pending payments database tree");
        info!("Opened database in \"{}/\"", db_path);
        let viewpair = monero::ViewPair {
            view: private_viewkey,
            spend: public_spendkey,
        };

        PaymentGateway(Arc::new(PaymentGatewayInner {
            daemon_url: self.daemon_url,
            viewpair,
            scan_rate,
            payments_db,
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
    owned_outputs: Vec<OwnedOutput>,
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
            owned_outputs: Vec::new(),
        }
    }

    pub fn is_confirmed(&self) -> bool {
        if let Some(confirmations) = self.confirmations() {
            confirmations >= self.confirmations_required
        } else {
            false
        }
    }

    pub fn is_expired(&self) -> bool {
        // At or passed the expiration block, AND not paid in full.
        self.current_height >= self.expiration_at && self.paid_at.is_none()
    }

    pub fn address(&self) -> String {
        self.address.clone()
    }

    pub fn index(&self) -> SubIndex {
        self.index
    }

    pub fn started_at(&self) -> u64 {
        self.started_at
    }

    pub fn amount_requested(&self) -> u64 {
        self.amount_requested
    }

    pub fn amount_paid(&self) -> u64 {
        self.amount_paid
    }

    pub fn confirmations_required(&self) -> u64 {
        self.confirmations_required
    }

    pub fn confirmations(&self) -> Option<u64> {
        if let Some(paid_at) = self.paid_at {
            Some(self.current_height.saturating_sub(paid_at) + 1)
        } else {
            None
        }
    }

    pub fn current_height(&self) -> u64 {
        self.current_height
    }

    pub fn expiration_at(&self) -> u64 {
        self.expiration_at
    }
}

#[derive(Debug, Copy, Clone, Hash, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubIndex {
    pub major: u32,
    pub minor: u32,
}

impl SubIndex {
    pub fn new(major: u32, minor: u32) -> SubIndex {
        SubIndex { major, minor }
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
pub struct OwnedOutput {
    pub amount: u64,
    pub height: Option<u64>,
}

impl OwnedOutput {
    fn new(amount: u64, height: Option<u64>) -> OwnedOutput {
        OwnedOutput { amount, height }
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
