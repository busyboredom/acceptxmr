mod util;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::mpsc::{channel, Sender};
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
    scanthread_tx: Option<Sender<String>>,
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
        let (tx, rx) = channel();
        self.scanthread_tx = Some(tx);

        // Spawn the scanning thread.
        thread::spawn(move || {
            // The thread needs a tokio runtime to process async functions.
            let tokio_runtime = Runtime::new().unwrap();
            tokio_runtime.block_on(async move {
                // Initially, there are no payments to track.
                let mut payments = HashMap::new();

                // Scan for transactions once every scan_rate.
                let mut blockscan_interval = time::interval(time::Duration::from_millis(scan_rate));
                loop {
                    blockscan_interval.tick().await;

                    // Check for new messages.
                    for message in rx.try_iter() {
                        println!("{}", message);
                    }

                    // Get transactions to scan.
                    let current_height = util::get_current_height(&url).await.unwrap();
                    println!("Current Block: {}", current_height);
                    let block = util::get_block(&url, current_height).await.unwrap();
                    let transactions = util::get_block_transactions(&url, block).await.unwrap();

                    // Scan the transactions.
                    util::scan_transactions(&viewpair, &mut payments, transactions);
                }
            })
        });
    }

    pub fn track_payment(&self, payment: &str) {
        match &self.scanthread_tx {
            Some(tx) => tx.send(payment.to_string()).unwrap(),
            None => panic!(
                "Can't communicate with scan thread; did you remember to run this blockscanner?"
            ),
        }
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
        util::scan_transactions(&self.viewpair, &mut self.payments, transactions)
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
        }
    }
}

pub struct Payment {
    pub payment_id: PaymentId,
    pub expected_amount: u64,
    pub paid_amount: u64,
    pub confirmations_required: u64,
    pub confirmations_recieved: u64,
    pub expiration_block: u64,
}

impl Payment {
    pub fn new(amount: u64, confirmations: u64, expiration_block: u64) -> Payment {
        Payment {
            payment_id: PaymentId::random(),
            expected_amount: amount,
            paid_amount: 0,
            confirmations_required: confirmations,
            confirmations_recieved: 0,
            expiration_block: expiration_block,
        }
    }
}
