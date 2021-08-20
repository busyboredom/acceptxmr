use std::str::FromStr;

use monero::network::Network;
use monero::util::{address, key};

pub struct BlockScanner {
    daemon_url: String,
    viewpair: key::ViewPair,
    scan_rate: u64,
}

impl BlockScanner {
    pub fn builder() -> BlockScannerBuilder {
        BlockScannerBuilder::default()
    }

    pub fn new_integrated_address(&self) -> (String, [u8; 8]) {
        let standard_address = address::Address::from_viewpair(Network::Mainnet, &self.viewpair);

        let integrated_address = address::Address::integrated(
            Network::Mainnet,
            standard_address.public_spend,
            standard_address.public_view,
            address::PaymentId::random(),
        );
        let payment_id = match integrated_address.addr_type {
            address::AddressType::Integrated(id) => id,
            _ => panic!("Integrated address malformed (no payment ID)"),
        };

        (format!("{}", integrated_address), payment_id.to_fixed_bytes())
    }
}

#[derive(Default)]
pub struct BlockScannerBuilder {
    daemon_url: String,
    private_viewkey: Option<key::PrivateKey>,
    public_spendkey: Option<key::PublicKey>,
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
            Some(key::PrivateKey::from_str(&private_viewkey).expect("Invalid private viewkey"));
        self
    }

    pub fn public_spendkey(mut self, public_spendkey: &str) -> BlockScannerBuilder {
        self.public_spendkey =
            Some(key::PublicKey::from_str(&public_spendkey).expect("Invalid public spendkey"));
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
        let viewpair = key::ViewPair {
            view: private_viewkey,
            spend: public_spendkey,
        };
        BlockScanner {
            daemon_url: self.daemon_url,
            viewpair: viewpair,
            scan_rate: scan_rate,
        }
    }
}

struct Payment {
    payment_id: address::PaymentId,
    expected_amount: u64,
    paid_amount: u64,
    confirmations_required: u64,
    confirmations_recieved: u64,
    expiration_block: u64,
}
