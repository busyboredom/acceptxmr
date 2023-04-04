#![warn(clippy::pedantic)]

use std::collections::{
    btree_map::{self, Entry},
    BTreeMap,
};

use acceptxmr::{storage::InvoiceStorage, Invoice, InvoiceId, PaymentGatewayBuilder, SubIndex};
use log::{error, info, LevelFilter};
use thiserror::Error;

#[tokio::main]
async fn main() {
    env_logger::builder()
        .filter_level(LevelFilter::Warn)
        .filter_module("acceptxmr", log::LevelFilter::Debug)
        .filter_module("custom_storage", log::LevelFilter::Trace)
        .init();

    // The private view key should be stored securely outside of the git repository.
    // It is hardcoded here for demonstration purposes only.
    let private_view_key = "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
    // No need to keep the primary address secret.
    let primary_address = "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";

    let payment_gateway = PaymentGatewayBuilder::new(
        private_view_key.to_string(),
        primary_address.to_string(),
        MyCustomStorage::new(), // Use your custom storage layer!
    )
    .daemon_url("http://node.sethforprivacy.com:18089".to_string())
    .build()
    .unwrap();

    info!("Payment gateway created.");

    // Any invoices created with this payment gateway will now be stored in your
    // custom storage layer.
    let invoice_id = payment_gateway
        .new_invoice(1000, 2, 5, "Demo invoice".to_string())
        .unwrap();
    let invoice = payment_gateway
        .get_invoice(invoice_id)
        .unwrap()
        .expect("invoice not found");

    info!(
        "Invoice retrieved from custom storage layer! address: \n\n{}\n",
        invoice.address()
    );
}

// This example uses a BTreeMap for simplicity, but you can implement this trait
// on virtually any storage layer you choose. Postgres or MySQL, CSV files,
// whatever works best for your application.
struct MyCustomStorage(BTreeMap<InvoiceId, Invoice>);

impl MyCustomStorage {
    /// Create a new custom invoice store.
    #[must_use]
    pub fn new() -> MyCustomStorage {
        MyCustomStorage(BTreeMap::new())
    }
}

impl Default for MyCustomStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl InvoiceStorage for MyCustomStorage {
    type Error = MyCustomStorageError;
    type Iter<'a> = MyCustomStorageIter<'a>;

    fn insert(&mut self, invoice: Invoice) -> Result<(), Self::Error> {
        if self.0.contains_key(&invoice.id()) {
            return Err(MyCustomStorageError::DuplicateEntry);
        }
        self.0.insert(invoice.id(), invoice);
        Ok(())
    }

    fn remove(&mut self, invoice_id: InvoiceId) -> Result<Option<Invoice>, Self::Error> {
        Ok(self.0.remove(&invoice_id))
    }

    fn update(&mut self, invoice: Invoice) -> Result<Option<Invoice>, Self::Error> {
        if let Entry::Occupied(mut entry) = self.0.entry(invoice.id()) {
            return Ok(Some(entry.insert(invoice)));
        }
        Ok(None)
    }

    fn get(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, Self::Error> {
        Ok(self.0.get(&invoice_id).cloned())
    }

    fn contains_sub_index(&self, sub_index: SubIndex) -> Result<bool, Self::Error> {
        Ok(self
            .0
            .range(InvoiceId::new(sub_index, 0)..)
            .next()
            .is_some())
    }

    fn try_iter(&self) -> Result<Self::Iter<'_>, Self::Error> {
        let iter = self.0.values();
        Ok(MyCustomStorageIter(iter))
    }
}

pub struct MyCustomStorageIter<'a>(btree_map::Values<'a, InvoiceId, Invoice>);

impl<'a> Iterator for MyCustomStorageIter<'a> {
    type Item = Result<Invoice, MyCustomStorageError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|v| Ok(v.clone()))
    }
}

/// An error occurring while storing or retrieving pending invoices.
#[derive(Error, Debug)]
#[error("BTreeMap invoice storage error")]
pub enum MyCustomStorageError {
    /// Attempted to insert an invoice which already exists
    #[error("attempted to insert an invoice which already exists")]
    DuplicateEntry,
}
