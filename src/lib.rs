use std::collections::HashMap;
use std::str::FromStr;

use monero::blockdata::transaction::{ExtraField, SubField};
use monero::consensus::encode::deserialize;
use monero::cryptonote::hash::keccak_256;
use monero::util::address::PaymentId;
use monero::Network::Mainnet;
use reqwest;
use serde_json;

pub struct BlockScanner {
    daemon_url: String,
    viewpair: monero::ViewPair,
    payments: HashMap<PaymentId, Payment>,
    scan_rate: u64,
}

impl BlockScanner {
    pub fn builder() -> BlockScannerBuilder {
        BlockScannerBuilder::default()
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
        let client = reqwest::Client::new();

        let request_body = r#"{"jsonrpc":"2.0","id":"0","method":"get_block","params":{"height":"#
            .to_owned()
            + &height.to_string()
            + "}}";
        let res = client
            .post(self.daemon_url.to_owned() + "/json_rpc")
            .body(request_body)
            .send()
            .await?;

        let block_json = res.json::<serde_json::Value>().await?;
        println!(
            "Block request status: {}",
            block_json["result"]["status"].as_str().unwrap()
        );

        let block_blob = block_json["result"]["blob"]
            .as_str()
            .expect("Failed to read block blob from json_rpc");

        let block_bytes =
            hex::decode(block_blob).expect("Failed to decode block blob from hex to bytes");

        let block = deserialize(&block_bytes).expect("Failed to deserialize block blob");

        Ok(block)
    }

    pub async fn get_block_transactions(
        &self,
        block: monero::Block,
    ) -> Result<Vec<monero::Transaction>, reqwest::Error> {
        // Get block transactions in sets of 100 or less (the restriced RPC maximum).
        let client = reqwest::Client::new();
        let mut transactions = vec![];
        let transaction_hashes = block.tx_hashes;
        println!("Transactions to fetch: {}", transaction_hashes.len());
        for i in 0..(transaction_hashes.len() / 100 + 1) {
            // Start and end indexes of the hashes we're grabbing for now.
            let starting_index: usize = i * 100;
            let ending_index: usize = std::cmp::min(100 * (i + 1), transaction_hashes.len());
            println!("Transactions requested: {}", ending_index - starting_index);

            // Build a json containing the hashes of the transactions we want.
            let request_body = r#"{"txs_hashes":"#.to_owned()
                + &serde_json::json!(transaction_hashes[starting_index..ending_index]
                    .iter()
                    .map(|x| hex::encode(x.as_bytes())) // Convert from monero::Hash to hex.
                    .collect::<Vec<String>>())
                .to_string()
                + "}";
            let res = client
                .post("http://busyboredom.com:18081/get_transactions")
                .body(request_body)
                .send()
                .await?;

            let res = res.json::<serde_json::Value>().await?;
            println!(
                "Transaction request status: {}",
                res["status"].as_str().unwrap()
            );

            // Add these transactions to the total list.
            if let Some(hexes) = res["txs_as_hex"].as_array() {
                for tx_json in hexes {
                    let tx_str = tx_json
                        .as_str()
                        .expect("Failed to read transaction hex from json");
                    let tx_hex = hex::decode(tx_str)
                        .expect("Failed to decode transaction fron hex to bytes");
                    let tx = deserialize(&tx_hex).expect("Failed to deserialize transaction");
                    transactions.push(tx);
                }
            }
        }

        println!("Transactions fetched: {}", transactions.len());

        Ok(transactions)
    }

    pub async fn scan_transactions(&mut self, transactions: Vec<monero::Transaction>) {
        for tx in transactions {
            let mut payment_id = PaymentId::zero();

            // Get owned outputs.
            let owned_outputs = tx.check_outputs(&self.viewpair, 0..1, 0..1).unwrap();

            // Generate and display the SubFields (the parsed "extra" section) if applicable.
            if owned_outputs.len() == 1 {
                // Payments to integrated addresses only ever have one output.

                // Get transaction's "extra" section.
                let tx_extra = &tx.prefix().extra;

                // Get vec of subfields from transaction's "extra" section.
                let ExtraField(subfields) = tx_extra;

                for subfield in subfields {
                    if let SubField::Nonce(nonce_bytes) = subfield {
                        // Shared secret can be retrieved as a combination of tx public key and your private view key.
                        let shared_secret = tx.tx_pubkey().unwrap() * &(self.viewpair.view * 8u8);

                        // The payment ID decryption key is a hash of the shared secret.
                        let mut key_bytes = shared_secret.as_bytes().to_vec();
                        key_bytes.append(&mut hex::decode("8d").unwrap());
                        let key = keccak_256(&key_bytes);

                        // The first byte of the nonce is not part of the encrypted payment ID.
                        let mut id_bytes = nonce_bytes.clone()[1..9].to_vec();

                        // Decrypt the payment ID by XORing it with the key.
                        id_bytes
                            .iter_mut()
                            .zip(key.iter())
                            .for_each(|(x1, x2)| *x1 ^= *x2);

                        payment_id = PaymentId::from_slice(&id_bytes);
                        println!("Payment ID: {}", hex::encode(&payment_id.as_bytes()))
                    }
                }
            }

            // If this payment is being tracked, update the amount paid.
            if let Some(payment) = self.payments.get_mut(&payment_id) {
                payment.paid_amount += owned_outputs[0]
                    .amount()
                    .expect("Failed to unblind transaction amount");
            }
        }
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
