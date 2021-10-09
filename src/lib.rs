mod block_cache;
mod error;
mod payments_db;
mod scanner;
mod subscriber;
mod util;

use std::cmp;
use std::str::FromStr;
use std::sync::{atomic, Arc};
use std::{fmt, thread, u64};

use log::{debug, info};
use monero::cryptonote::subaddress;
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;
use tokio::{join, time};

use block_cache::BlockCache;
use error::Error;
use payments_db::PaymentsDb;
use scanner::Scanner;
pub use subscriber::Subscriber;

pub struct PaymentGateway {
    daemon_url: String,
    viewpair: monero::ViewPair,
    scan_rate: u64,
    payments_db: PaymentsDb,
    height: Arc<atomic::AtomicU64>,
}

impl PaymentGateway {
    pub fn builder() -> PaymentGatewayBuilder {
        PaymentGatewayBuilder::default()
    }

    pub fn run(&mut self, cache_size: u64) {
        // Gather info needed by the scanner.
        let url = self.daemon_url.to_owned();
        let viewpair = monero::ViewPair {
            view: self.viewpair.view,
            spend: self.viewpair.spend,
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
    pub fn new_payment(
        &mut self,
        xmr: f64,
        confirmations_required: u64,
        expiration_in: u64,
    ) -> Result<Subscriber, Error> {
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
        Ok(self.watch_payment(sub_index))
    }

    pub fn watch_payment(&self, sub_index: SubIndex) -> Subscriber {
        self.payments_db.watch_payment(sub_index)
    }

    pub async fn get_daemon_height(&self) -> Result<u64, Error> {
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
        info!("Opened database in \"{}\"/", db_path);
        let viewpair = monero::ViewPair {
            view: private_viewkey,
            spend: public_spendkey,
        };
        PaymentGateway {
            daemon_url: self.daemon_url,
            viewpair,
            scan_rate,
            payments_db,
            height: Arc::new(atomic::AtomicU64::new(0)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Payment {
    pub address: String,
    pub index: SubIndex,
    pub starting_block: u64,
    pub expected_amount: u64,
    pub paid_amount: u64,
    pub paid_at: Option<u64>,
    pub confirmations_required: u64,
    pub current_block: u64,
    pub expiration_block: u64,
    pub owned_outputs: Vec<OwnedOutput>,
}

impl Payment {
    pub fn new(
        address: &str,
        index: SubIndex,
        starting_block: u64,
        amount: u64,
        confirmations_required: u64,
        expiration_in: u64,
    ) -> Payment {
        let expiration_block = starting_block + expiration_in;
        Payment {
            address: address.to_string(),
            index,
            starting_block,
            expected_amount: amount,
            paid_amount: 0,
            paid_at: None,
            confirmations_required,
            current_block: 0,
            expiration_block,
            owned_outputs: Vec::new(),
        }
    }

    pub fn is_confirmed(&self) -> bool {
        match self.paid_at {
            Some(height) => {
                let confirmations = self.current_block.saturating_sub(height) + 1;
                confirmations >= self.confirmations_required
            }
            None => false,
        }
    }

    pub fn is_expired(&self) -> bool {
        // At or passed the expiration block, AND not paid in full.
        self.current_block >= self.expiration_block && self.paid_at.is_none()
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
    amount: u64,
    height: Option<u64>,
}

impl OwnedOutput {
    pub fn new(amount: u64, height: Option<u64>) -> OwnedOutput {
        OwnedOutput { amount, height }
    }

    pub fn newer_than(&self, other_height: u64) -> bool {
        match self.height {
            Some(h) => h > other_height,
            None => true,
        }
    }

    pub fn older_than(&self, other_height: u64) -> bool {
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
