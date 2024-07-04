use std::collections::{btree_map::Entry, BTreeMap};

use thiserror::Error;

use crate::{
    storage::{HeightStorage, InvoiceStorage, OutputId, OutputKeyStorage, OutputPubKey, Storage},
    Invoice, InvoiceId, SubIndex,
};

/// In-memory store. Note that invoices stored in memory will not be recoverable
/// on power loss. [Burning
/// bug](https://www.getmonero.org/2018/09/25/a-post-mortum-of-the-burning-bug.html)
/// mitigation will also be reset after application restart.
pub struct InMemory {
    invoices: BTreeMap<InvoiceId, Invoice>,
    output_keys: BTreeMap<OutputPubKey, OutputId>,
    height: Option<u64>,
}

impl InMemory {
    /// Create a new in-memory store.
    #[must_use]
    pub fn new() -> InMemory {
        InMemory {
            invoices: BTreeMap::new(),
            output_keys: BTreeMap::new(),
            height: None,
        }
    }
}

impl Default for InMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl InvoiceStorage for InMemory {
    type Error = InMemoryStorageError;

    fn insert(&mut self, invoice: Invoice) -> Result<(), Self::Error> {
        if self.invoices.contains_key(&invoice.id()) {
            return Err(InMemoryStorageError::DuplicateInvoice);
        }
        self.invoices.insert(invoice.id(), invoice);
        Ok(())
    }

    fn remove(&mut self, invoice_id: InvoiceId) -> Result<Option<Invoice>, Self::Error> {
        Ok(self.invoices.remove(&invoice_id))
    }

    fn update(&mut self, invoice: Invoice) -> Result<Option<Invoice>, Self::Error> {
        if let Entry::Occupied(mut entry) = self.invoices.entry(invoice.id()) {
            return Ok(Some(entry.insert(invoice)));
        }
        Ok(None)
    }

    fn get(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, Self::Error> {
        Ok(self.invoices.get(&invoice_id).cloned())
    }

    fn get_ids(&self) -> Result<Vec<InvoiceId>, Self::Error> {
        Ok(self.invoices.keys().copied().collect::<Vec<InvoiceId>>())
    }

    fn contains_sub_index(&self, sub_index: SubIndex) -> Result<bool, Self::Error> {
        Ok(self
            .invoices
            .range(InvoiceId::new(sub_index, 0)..)
            .next()
            .is_some())
    }

    fn try_for_each<F>(&self, mut f: F) -> Result<(), Self::Error>
    where
        F: FnMut(Result<Invoice, Self::Error>) -> Result<(), Self::Error>,
    {
        self.invoices
            .iter()
            .try_for_each(move |(_, invoice)| f(Ok(invoice.clone())))
    }

    fn is_empty(&self) -> Result<bool, Self::Error> {
        Ok(self.invoices.is_empty())
    }
}

impl OutputKeyStorage for InMemory {
    type Error = InMemoryStorageError;

    fn insert(&mut self, key: OutputPubKey, output_id: OutputId) -> Result<(), Self::Error> {
        if self.output_keys.contains_key(&key) {
            return Err(InMemoryStorageError::DuplicateOutputKey);
        }
        self.output_keys.insert(key, output_id);
        Ok(())
    }

    fn get(&self, key: OutputPubKey) -> Result<Option<OutputId>, Self::Error> {
        Ok(self.output_keys.get(&key).copied())
    }
}

impl HeightStorage for InMemory {
    type Error = InMemoryStorageError;

    fn upsert(&mut self, height: u64) -> Result<Option<u64>, Self::Error> {
        let old_height = self.height;
        self.height = Some(height);
        Ok(old_height)
    }

    fn get(&self) -> Result<Option<u64>, Self::Error> {
        Ok(self.height)
    }
}

impl Storage for InMemory {
    type Error = InMemoryStorageError;
}

/// An error occurring while storing or retrieving values in memory.
#[derive(Error, Debug)]
#[error("in-memory invoice storage error")]
pub enum InMemoryStorageError {
    /// Attempted to insert an invoice which already exists
    #[error("attempted to insert an invoice which already exists")]
    DuplicateInvoice,
    /// Attempted to insert an output public key which already exists
    #[error("attempted to insert an output public key which already exists")]
    DuplicateOutputKey,
}
