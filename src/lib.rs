mod block_cache;
mod error;
mod payments_db;
mod scanner;
mod util;

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::{thread, u64};

use log::info;
use monero::cryptonote::subaddress;
use serde::{Serialize, Deserialize};
use tokio::runtime::Runtime;
use tokio::{join, time};

use block_cache::BlockCache;
use error::Error;
use scanner::Scanner;
use payments_db::PaymentsDb;

//#[derive(Debug, Clone)]
pub struct PaymentProcessor {
    daemon_url: String,
    viewpair: monero::ViewPair,
    payments: HashMap<SubIndex, Payment>,
    scan_rate: u64,
    scanner_tx: Option<Sender<Payment>>,
    scanner_rx: Option<Receiver<Receiver<Payment>>>,
}

impl PaymentProcessor {
    pub fn builder() -> PaymentProcessorBuilder {
        PaymentProcessorBuilder::default()
    }

    pub fn run(&mut self, cache_size: u64, initial_height: u64) {
        // Gather info needed by the scanner.
        let url = self.daemon_url.to_owned();
        let viewpair = monero::ViewPair {
            view: self.viewpair.view,
            spend: self.viewpair.spend,
        };
        let scan_rate = self.scan_rate;

        // Set up communication with the scanner.
        let (payment_tx, payment_rx) = channel();
        let (channel_tx, channel_rx) = channel();
        self.scanner_tx = Some(payment_tx);
        self.scanner_rx = Some(channel_rx);

        // Spawn the scanning thread.
        info!("Starting blockchain scanner now.");
        thread::spawn(move || {
            // The thread needs a tokio runtime to process async functions.
            let tokio_runtime = Runtime::new().unwrap();
            tokio_runtime.block_on(async move {
                // Initially, there are no payments to track.
                let payments = HashMap::new();

                // Open (or create) db of pending payments.
                let pending_payments = PaymentsDb::new(sled::open("PaymentsDb").unwrap());

                // For each payment, we need a channel to send updates back to the initiating thread.
                let channels = HashMap::new();

                // Keep a cache of blocks.
                let block_cache = BlockCache::init(&url, cache_size, initial_height)
                    .await
                    .unwrap();

                // Create scanner.
                let mut scanner = Scanner::new(
                    url,
                    viewpair,
                    payment_rx,
                    channel_tx,
                    payments,
                    pending_payments,
                    channels,
                    block_cache,
                );

                // Scan for transactions once every scan_rate.
                let mut blockscan_interval = time::interval(time::Duration::from_millis(scan_rate));
                loop {
                    join!(blockscan_interval.tick(), scanner.scan());
                    scanner.track_new_payments();
                }
            })
        });
    }

    pub fn track_payment(&self, payment: Payment) -> Receiver<Payment> {
        if self.scanner_rx.is_none() || self.scanner_tx.is_none() {
            panic!("Can't communicate with scan thread; did you remember to run this PaymentProcessor?")
        }

        // Send the payment to the scanning thread.
        self.scanner_tx.as_ref().unwrap().send(payment).unwrap();

        // Return a reciever so the caller can get updates on payment status.
        self.scanner_rx.as_ref().unwrap().recv().unwrap()
    }

    pub fn new_subaddress(&self) -> (String, SubIndex) {
        let subindex = SubIndex::new(0, 1);
        let subaddress = subaddress::get_subaddress(&self.viewpair, subindex.into(), None);
        // Return address in base58, and payment ID in hex.
        (format!("{}", subaddress), subindex)
    }

    pub async fn get_block(&self, height: u64) -> Result<(monero::Hash, monero::Block), Error> {
        util::get_block(&self.daemon_url, height).await
    }

    pub async fn get_block_transactions(
        &self,
        block: monero::Block,
    ) -> Result<Vec<monero::Transaction>, Error> {
        util::get_block_transactions(&self.daemon_url, &block).await
    }

    pub fn scan_transactions(&mut self, transactions: Vec<monero::Transaction>) {
        util::scan_transactions(&self.viewpair, &self.payments, transactions);
    }

    pub async fn get_current_height(&self) -> Result<u64, Error> {
        util::get_current_height(&self.daemon_url).await
    }
}

#[derive(Default)]
pub struct PaymentProcessorBuilder {
    daemon_url: String,
    private_viewkey: Option<monero::PrivateKey>,
    public_spendkey: Option<monero::PublicKey>,
    scan_rate: Option<u64>,
}

impl PaymentProcessorBuilder {
    pub fn new() -> PaymentProcessorBuilder {
        PaymentProcessorBuilder::default()
    }

    pub fn daemon_url(mut self, url: &str) -> PaymentProcessorBuilder {
        reqwest::Url::parse(url).expect("Invalid daemon URL");
        self.daemon_url = url.to_string();
        self
    }

    pub fn private_viewkey(mut self, private_viewkey: &str) -> PaymentProcessorBuilder {
        self.private_viewkey =
            Some(monero::PrivateKey::from_str(private_viewkey).expect("Invalid private viewkey"));
        self
    }

    pub fn public_spendkey(mut self, public_spendkey: &str) -> PaymentProcessorBuilder {
        self.public_spendkey =
            Some(monero::PublicKey::from_str(public_spendkey).expect("Invalid public spendkey"));
        self
    }

    pub fn scan_rate(mut self, milliseconds: u64) -> PaymentProcessorBuilder {
        self.scan_rate = Some(milliseconds);
        self
    }

    pub fn build(self) -> PaymentProcessor {
        let private_viewkey = self
            .private_viewkey
            .expect("Private viewkey must be defined");
        let public_spendkey = self
            .public_spendkey
            .expect("Private viewkey must be defined");
        let scan_rate = self.scan_rate.unwrap_or(1000);
        let viewpair = monero::ViewPair {
            view: private_viewkey,
            spend: public_spendkey,
        };
        PaymentProcessor {
            daemon_url: self.daemon_url,
            viewpair,
            payments: HashMap::new(),
            scan_rate,
            scanner_tx: None,
            scanner_rx: None,
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
    // Partial payments take the form (block height, amount).
    pub partial_payments: Vec<(u64, u64)>
}

impl Payment {
    pub fn new(
        address: &str,
        index: SubIndex,
        starting_block: u64,
        amount: u64,
        confirmations: u64,
        expiration_block: u64,
    ) -> Payment {
        Payment {
            address: address.to_string(),
            index,
            starting_block,
            expected_amount: amount,
            paid_amount: 0,
            paid_at: None,
            confirmations_required: confirmations,
            current_block: 0,
            expiration_block,
            partial_payments: Vec::new(),
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
