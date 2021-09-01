mod block_cache;
mod error;
mod util;

use std::collections::HashMap;
use std::convert::TryInto;
use std::fmt;
use std::str::FromStr;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::{thread, u64};

use log::{debug, error, info, trace, warn};
use monero::cryptonote::subaddress;
use reqwest;
use serde::Serialize;
use tokio::runtime::Runtime;
use tokio::time;

use block_cache::BlockCache;
use error::Error;

//#[derive(Debug, Clone)]
pub struct PaymentProcessor {
    daemon_url: String,
    viewpair: monero::ViewPair,
    payments: HashMap<SubIndex, Payment>,
    scan_rate: u64,
    scanthread_tx: Option<Sender<Payment>>,
    scanthread_rx: Option<Receiver<Receiver<Payment>>>,
}

impl PaymentProcessor {
    pub fn builder() -> PaymentProcessorBuilder {
        PaymentProcessorBuilder::default()
    }

    pub fn run(&mut self, cache_size: u64, initial_height: u64) {
        // Gather info needed by the scanning thread.
        let url = self.daemon_url.to_owned();
        let viewpair = monero::ViewPair {
            view: self.viewpair.view.clone(),
            spend: self.viewpair.spend.clone(),
        };
        let scan_rate = self.scan_rate;

        // Set up communication with the scanning thread.
        let (main_tx, thread_rx) = channel();
        let (thread_tx, main_rx) = channel();
        self.scanthread_tx = Some(main_tx);
        self.scanthread_rx = Some(main_rx);

        // Spawn the scanning thread.
        info!("Starting blockchain scanner now.");
        thread::spawn(move || {
            // The thread needs a tokio runtime to process async functions.
            let tokio_runtime = Runtime::new().unwrap();
            tokio_runtime.block_on(async move {
                // Initially, there are no payments to track.
                let mut payments = HashMap::new();

                // For each payment, we need a channel to send updates back to the initiating thread.
                let mut channels = HashMap::new();

                // Keep a cache of blocks.
                let mut block_cache = BlockCache::init(&url, cache_size, initial_height)
                    .await
                    .unwrap();

                // Scan for transactions once every scan_rate.
                let mut blockscan_interval = time::interval(time::Duration::from_millis(scan_rate));
                loop {
                    blockscan_interval.tick().await;

                    // Check for new payments to track.
                    for payment in thread_rx.try_iter() {
                        // Add the payment to the hashmap for tracking.
                        payments.insert(payment.index, payment.clone());

                        // Set up communication for sending updates on this payment.
                        let (payment_tx, payment_rx) = channel();
                        channels.insert(payment.index, payment_tx);
                        thread_tx.send(payment_rx).unwrap();

                        debug!("Now tracking subaddress index {}", payment.index);
                    }

                    // Update block cache and txpool.
                    if let Err(e) = block_cache.update(&url).await {
                        error!("Faled to update block cache: {}", e);
                    }

                    let txpool = match util::get_txpool(&url).await {
                        Ok (pool) => pool,
                        Err(e) => {
                            error!("Faled to get transaction pool: {}", e);
                            Vec::new()
                        }
                    };

                    let current_height = match util::get_current_height(&url).await {
                        Ok(height) => {
                            trace!(
                                "Cache height: {}, Blockchain height: {}",
                                block_cache.height,
                                height
                            );
                            height
                        }
                        Err(e) => {
                            error!("Faled to get current height: {}", e);
                            block_cache.height
                        }
                    };

                    // Set up temporary payments hashmap so we'll know which ones are updated.
                    let mut updated_payments = payments.clone();
                    for payment in updated_payments.values_mut() {
                        payment.paid_amount = 0;
                        payment.paid_at = None;
                    }

                    // Scan blocks.
                    for i in (0..block_cache.blocks.len()).rev() {
                        let transactions = &block_cache.blocks[i].3;
                        let amounts_received =
                            util::scan_transactions(&viewpair, &payments, transactions.to_vec());
                        trace!(
                            "Scanned {} transactions from block {}, and found {} tracked payments.",
                            transactions.len(),
                            block_cache.blocks[i].1,
                            amounts_received.len()
                        );

                        // Update payment amounts.
                        for (&subindex, amount) in amounts_received.iter() {
                            let payment = updated_payments.get_mut(&subindex).unwrap();
                            payment.paid_amount += amount;

                            if payment.paid_amount >= payment.expected_amount
                                && payment.paid_at.is_none()
                            {
                                let pos_in_cache: u64 = i.try_into().unwrap();
                                let height = block_cache.height - pos_in_cache;
                                payment.paid_at = Some(height);
                            }
                        }
                    }

                    // Scan txpool.
                    let amounts_received =
                        util::scan_transactions(&viewpair, &payments, txpool.to_vec());
                    trace!(
                        "Scanned {} transactions from txpool, and found {} tracked payments.",
                        txpool.len(),
                        amounts_received.len()
                    );
                    // Update payment amounts.
                    for (&payment_id, amount) in amounts_received.iter() {
                        let payment = updated_payments.get_mut(&payment_id).unwrap();
                        payment.paid_amount += amount;

                        if payment.paid_amount >= payment.expected_amount
                            && payment.paid_at.is_none()
                        {
                            payment.paid_at = Some(current_height + 1);
                        }
                    }

                    // Send updates and mark completed/expired payments for removal,.
                    let mut completed = Vec::new();
                    for payment in updated_payments.values_mut() {
                        // Update payment's current_block.
                        if block_cache.height != payment.current_block {
                            payment.current_block = block_cache.height;
                        }
                        // If payment was updated, send an update.
                        if payment != payments.get(&payment.index).unwrap() {
                            if let Err(_) =
                                channels.get(&payment.index).unwrap().send(payment.clone())
                            {
                                warn!("Receiver disconnected before payment completed.");
                                completed.push(payment.index);
                            }
                            // Copy the updated payment parameters to the main one.
                            payments.insert(payment.index, payment.clone());
                        }
                        // If the payment is fully confirmed (and at least a block old), mark it as complete.
                        if payment.is_confirmed() && payment.starting_block < current_height {
                            completed.push(payment.index);
                        }
                        // If the payment has expired, mark it as complete.
                        if payment.is_expired() {
                            completed.push(payment.index);
                        }
                    }
                    // Stop tracking completed/expired transactions.
                    for index in completed {
                        if payments.remove(&index).is_none() {
                            warn!("Attempted to remove subaddress index {} from tracked payments, but it didn't exist.", index);
                        }
                        if channels.remove(&index).is_none() {
                            warn!("Attempted to remove subaddress index {} from payment update channels, but it didn't exist.", index);
                        }
                        debug!("No longer tracking subaddress index {}", index);
                    }
                }
            })
        });
    }

    pub fn track_payment(&self, payment: Payment) -> Receiver<Payment> {
        if self.scanthread_rx.is_none() || self.scanthread_tx.is_none() {
            panic!("Can't communicate with scan thread; did you remember to run this PaymentProcessor?")
        }

        // Send the payment to the scanning thread.
        self.scanthread_tx.as_ref().unwrap().send(payment).unwrap();

        // Return a reciever so the caller can get updates on payment status.
        self.scanthread_rx.as_ref().unwrap().recv().unwrap()
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
        util::scan_transactions(&self.viewpair, &mut self.payments, transactions);
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
            Some(monero::PrivateKey::from_str(&private_viewkey).expect("Invalid private viewkey"));
        self
    }

    pub fn public_spendkey(mut self, public_spendkey: &str) -> PaymentProcessorBuilder {
        self.public_spendkey =
            Some(monero::PublicKey::from_str(&public_spendkey).expect("Invalid public spendkey"));
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
            viewpair: viewpair,
            payments: HashMap::new(),
            scan_rate: scan_rate,
            scanthread_tx: None,
            scanthread_rx: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
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
        }
    }

    pub fn is_confirmed(&self) -> bool {
        match self.paid_at {
            Some(height) => {
                let confirmations = self.current_block.saturating_sub(height) + 1;
                return confirmations >= self.confirmations_required;
            }
            None => return false,
        }
    }

    pub fn is_expired(&self) -> bool {
        // At or passed the expiration block, AND not paid in full.
        return self.current_block >= self.expiration_block && self.paid_at.is_none();
    }
}

#[derive(Debug, Copy, Clone, Hash, Serialize, PartialEq, Eq)]
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

impl Into<subaddress::Index> for SubIndex {
    fn into(self) -> subaddress::Index {
        subaddress::Index {
            major: self.major,
            minor: self.minor,
        }
    }
}
