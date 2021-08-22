mod util;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use monero::util::address::PaymentId;
use monero::Network::Mainnet;
use reqwest;
use tokio::runtime::Runtime;
use tokio::time;

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

    pub fn run(&mut self) {
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
        thread::spawn(move || {
            // The thread needs a tokio runtime to process async functions.
            let tokio_runtime = Runtime::new().unwrap();
            tokio_runtime.block_on(async move {
                // Initially, there are no payments to track.
                let mut payments = HashMap::new();

                // For each payment, we need a channel to send updates back to the initiating thread.
                let mut channels = HashMap::new();

                // Scan for transactions once every scan_rate.
                let mut blockscan_interval = time::interval(time::Duration::from_millis(scan_rate));
                loop {
                    blockscan_interval.tick().await;

                    // Check for new payments to track.
                    for payment in thread_rx.try_iter() {
                        // Add the payment to the hashmap for tracking.
                        println!("{:?}", payment);
                        payments.insert(payment.payment_id, payment);

                        // Set up communication for sending updates on this payment.
                        let (payment_tx, payment_rx) = channel();
                        channels.insert(payment.payment_id, payment_tx);
                        thread_tx.send(payment_rx).unwrap();
                    }

                    // Get transactions to scan.
                    let current_height = util::get_current_height(&url).await.unwrap();
                    println!("Current Block: {}", current_height);
                    let block = util::get_block(&url, current_height).await.unwrap();
                    let transactions = util::get_block_transactions(&url, block).await.unwrap();

                    // Scan the transactions.
                    let amounts_recieved =
                        util::scan_transactions(&viewpair, &payments, transactions);

                    // Send transaction updates, and mark completed/expired payments for removal.
                    let mut completed = Vec::new();
                    for payment in payments.values_mut() {
                        // If anything was recieved, send an update.
                        if let Some(amount) = amounts_recieved.get(&payment.payment_id) {
                            payment.paid_amount += amount;
                            channels.get(&payment.payment_id).unwrap().send(*payment).unwrap();
                        }
                        // If the payment is paid in full, mark it as complete.
                        if payment.paid_amount >= payment.expected_amount {
                            completed.push(payment.payment_id);
                        }
                    }
                    // Stop tracking completed/expired transactions.
                    for payment_id in completed {
                        payments.remove(&payment_id).unwrap();
                        channels.remove(&payment_id).unwrap();
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

    pub async fn get_block(&self, height: u64) -> Result<monero::Block, reqwest::Error> {
        util::get_block(&self.daemon_url, height).await
    }

    pub async fn get_block_transactions(
        &self,
        block: monero::Block,
    ) -> Result<Vec<monero::Transaction>, reqwest::Error> {
        util::get_block_transactions(&self.daemon_url, block).await
    }

    pub fn scan_transactions(&mut self, transactions: Vec<monero::Transaction>) {
        util::scan_transactions(&self.viewpair, &mut self.payments, transactions);
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

#[derive(Debug, Copy, Clone)]
pub struct Payment {
    pub payment_id: PaymentId,
    pub expected_amount: u64,
    pub paid_amount: u64,
    pub confirmations_required: u64,
    pub confirmations_recieved: u64,
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
            payment_id: payment_id,
            expected_amount: amount,
            paid_amount: 0,
            confirmations_required: confirmations,
            confirmations_recieved: 0,
            expiration_block: expiration_block,
        }
    }
}
