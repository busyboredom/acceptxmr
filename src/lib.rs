mod block_cache;
mod error;
mod util;

use std::collections::HashMap;
use std::convert::TryInto;
use std::str::FromStr;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::{thread, u64};

use log::{debug, info, trace};
use monero::util::address::PaymentId;
use monero::Network::Mainnet;
use reqwest;
use tokio::runtime::Runtime;
use tokio::time;

use block_cache::BlockCache;
use error::Error;

pub struct BlockScanner {
    daemon_url: String,
    viewpair: monero::ViewPair,
    payments: HashMap<PaymentId, Payment>,
    scan_rate: u64,
    scanthread_tx: Option<Sender<Payment>>,
    scanthread_rx: Option<Receiver<Receiver<Payment>>>,
}

impl BlockScanner {
    pub fn builder() -> BlockScannerBuilder {
        BlockScannerBuilder::default()
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
                        payments.insert(payment.payment_id, payment);

                        // Set up communication for sending updates on this payment.
                        let (payment_tx, payment_rx) = channel();
                        channels.insert(payment.payment_id, payment_tx);
                        thread_tx.send(payment_rx).unwrap();

                        debug!("Now tracking payment ID \"{}\"", payment.payment_id);
                    }

                    // Update block cache and txpool.
                    block_cache.update(&url).await.unwrap();
                    let txpool = util::get_txpool(&url).await.unwrap();

                    let current_height = util::get_current_height(&url).await.unwrap();
                    trace!(
                        "Cache height: {}, Blockchain height: {}",
                        block_cache.height,
                        current_height
                    );

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
                        for (&payment_id, amount) in amounts_received.iter() {
                            let payment = updated_payments.get_mut(&payment_id).unwrap();
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
                        if payment != payments.get(&payment.payment_id).unwrap() {
                            channels
                                .get(&payment.payment_id)
                                .unwrap()
                                .send(*payment)
                                .unwrap();
                            // Copy the updated payment parameters to the main one.
                            payments.insert(payment.payment_id, *payment);
                        }
                        // If the payment is fully confirmed, mark it as complete.
                        if let Some(paid_at) = payment.paid_at {
                            if payment.current_block >= paid_at + payment.confirmations_required - 1
                            {
                                completed.push(payment.payment_id);
                            }
                        }
                        // If the payment has expired, mark it as complete.
                        if payment.current_block >= payment.expiration_block {
                            completed.push(payment.payment_id);
                        }
                    }
                    // Stop tracking completed/expired transactions.
                    for payment_id in completed {
                        payments.remove(&payment_id).unwrap();
                        channels.remove(&payment_id).unwrap();
                        debug!("No longer tracking payment ID \"{}\"", payment_id);
                    }
                }
            })
        });
    }

    pub fn track_payment(&self, payment: Payment) -> Receiver<Payment> {
        if self.scanthread_rx.is_none() || self.scanthread_tx.is_none() {
            panic!("Can't communicate with scan thread; did you remember to run this blockscanner?")
        }

        // Send the payment to the scanning thread.
        self.scanthread_tx.as_ref().unwrap().send(payment).unwrap();

        // Return a reciever so the caller can get updates on payment status.
        self.scanthread_rx.as_ref().unwrap().recv().unwrap()
    }

    pub fn new_integrated_address(&self) -> (String, String) {
        let standard_address = monero::Address::from_viewpair(Mainnet, &self.viewpair);

        let integrated_address = monero::Address::integrated(
            Mainnet,
            standard_address.public_spend,
            standard_address.public_view,
            PaymentId::random(),
        );
        let payment_id = match integrated_address.addr_type {
            monero::AddressType::Integrated(id) => id,
            _ => panic!("Integrated address malformed (no payment ID)"),
        };

        // Return address in base58, and payment ID in hex.
        (format!("{}", integrated_address), hex::encode(&payment_id))
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

    pub async fn get_current_height(&mut self) -> Result<u64, Error> {
        util::get_current_height(&self.daemon_url).await
    }
}

#[derive(Default)]
pub struct BlockScannerBuilder {
    daemon_url: String,
    private_viewkey: Option<monero::PrivateKey>,
    public_spendkey: Option<monero::PublicKey>,
    scan_rate: Option<u64>,
}

impl BlockScannerBuilder {
    pub fn new() -> BlockScannerBuilder {
        BlockScannerBuilder::default()
    }

    pub fn daemon_url(mut self, url: &str) -> BlockScannerBuilder {
        reqwest::Url::parse(url).expect("Invalid daemon URL");
        self.daemon_url = url.to_string();
        self
    }

    pub fn private_viewkey(mut self, private_viewkey: &str) -> BlockScannerBuilder {
        self.private_viewkey =
            Some(monero::PrivateKey::from_str(&private_viewkey).expect("Invalid private viewkey"));
        self
    }

    pub fn public_spendkey(mut self, public_spendkey: &str) -> BlockScannerBuilder {
        self.public_spendkey =
            Some(monero::PublicKey::from_str(&public_spendkey).expect("Invalid public spendkey"));
        self
    }

    pub fn scan_rate(mut self, milliseconds: u64) -> BlockScannerBuilder {
        self.scan_rate = Some(milliseconds);
        self
    }

    pub fn build(self) -> BlockScanner {
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
        BlockScanner {
            daemon_url: self.daemon_url,
            viewpair: viewpair,
            payments: HashMap::new(),
            scan_rate: scan_rate,
            scanthread_tx: None,
            scanthread_rx: None,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Payment {
    pub payment_id: PaymentId,
    pub expected_amount: u64,
    pub paid_amount: u64,
    pub paid_at: Option<u64>,
    pub confirmations_required: u64,
    pub current_block: u64,
    pub expiration_block: u64,
}

impl Payment {
    pub fn new(
        payment_id: &str,
        amount: u64,
        confirmations: u64,
        expiration_block: u64,
    ) -> Payment {
        let payment_id =
            PaymentId::from_slice(&hex::decode(payment_id).expect("Invalid payment ID"));
        Payment {
            payment_id,
            expected_amount: amount,
            paid_amount: 0,
            paid_at: None,
            confirmations_required: confirmations,
            current_block: 0,
            expiration_block,
        }
    }
}
