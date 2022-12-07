//! `AcceptXMR` can store pending invoices using a storage layer of your choosing. Consumers of this
//! library can use one of the existing storage layers found in [`stores`], or can implement the
//! [`InvoiceStorage`] trait themselves for a custom storage solution.

pub mod stores;

use std::{
    cmp::Ordering,
    fmt::Display,
    sync::{Arc, PoisonError, RwLock, RwLockReadGuard},
};

use crate::{Invoice, InvoiceId, SubIndex};

/// The [`InvoiceStorage`] trait describes the storage layer for pending invoices. Consumers of this
/// library can use one of the existing storage layers found in [`stores`], or implement this trait
/// themselves for a custom storage solution.
pub trait InvoiceStorage: Send + Sync {
    /// Error type for the storage layer.
    type Error: Display + Send;
    /// An iterator over all invoices in storage.
    type Iter<'a>: Iterator<Item = Result<Invoice, Self::Error>>
    where
        Self: 'a;

    /// Insert invoice into storage for tracking, returning the previous value for that invoice if
    /// there was one.
    ///
    /// # Errors
    ///
    /// Returns an error if the invoice could not be inserted.
    fn insert(&mut self, invoice: Invoice) -> Result<Option<Invoice>, Self::Error>;

    /// Remove invoice from storage, returning the invoice if it existed.
    ///
    /// # Errors
    ///
    /// Returns an error if the invoice could not be removed.
    fn remove(&mut self, invoice_id: InvoiceId) -> Result<Option<Invoice>, Self::Error>;

    /// Update existing invoice in storage, returning old value if it existed. If the invoice does
    /// not already exist, does nothing.
    ///
    /// # Errors
    ///
    /// Returns an error if the invoice could not be updated.
    fn update(&mut self, invoice: Invoice) -> Result<Option<Invoice>, Self::Error>;

    /// Retrieve invoice from storage, returning `None` if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the invoice could not read.
    fn get(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, Self::Error>;

    /// Returns whether an invoice for the given subaddress exists in storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the existence of an invoice for the given subaddress could not be
    /// determined.
    fn contains_sub_index(&self, sub_index: SubIndex) -> Result<bool, Self::Error>;

    /// Returns an iterator over all invoices in storage.
    fn iter(&self) -> Self::Iter<'_>;

    /// Recover lowest current height of an invoice in storage. Scanning will resume from this
    /// height.
    ///
    /// # Errors
    ///
    /// Returns an error if the lowest height of an invoice could not be determined.
    fn lowest_height(&self) -> Result<Option<u64>, Self::Error> {
        self.iter()
            .min_by(|invoice_1, invoice_2| {
                match (invoice_1, invoice_2) {
                    // If there is an error, we want to return it.
                    (Err(_), _) => Ordering::Greater,
                    (_, Err(_)) => Ordering::Less,
                    // Otherwise, return the one with the lower height.
                    (Ok(inv1), Ok(inv2)) => inv1.current_height().cmp(&inv2.current_height()),
                }
            })
            .transpose()
            .map(|maybe_invoice| maybe_invoice.map(|invoice| invoice.current_height()))
    }

    /// Returns `true` if there are no invoices in storage.
    fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// Flush all changes to disk. This method should be manually implemented for any storage layer
    /// that does not automatically flush on write. The default implementation does nothing.
    ///
    /// # Errors
    ///
    /// Returns an error if flush does not succeed.
    fn flush(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

pub(crate) struct Store<S: InvoiceStorage>(Arc<RwLock<S>>);

impl<S: InvoiceStorage> Store<S> {
    pub fn new(store: S) -> Store<S> {
        Store(Arc::new(RwLock::new(store)))
    }

    pub fn insert(&self, invoice: Invoice) -> Result<Option<Invoice>, S::Error> {
        let mut store = self.0.write().unwrap_or_else(PoisonError::into_inner);
        store.insert(invoice)
    }

    pub fn remove(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, S::Error> {
        let mut store = self.0.write().unwrap_or_else(PoisonError::into_inner);
        store.remove(invoice_id)
    }

    pub fn update(&self, invoice: Invoice) -> Result<Option<Invoice>, S::Error> {
        let mut store = self.0.write().unwrap_or_else(PoisonError::into_inner);
        store.update(invoice)
    }

    pub fn get(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, S::Error> {
        let store = self.0.read().unwrap_or_else(PoisonError::into_inner);
        store.get(invoice_id)
    }

    pub fn contains_sub_index(&self, sub_index: SubIndex) -> Result<bool, S::Error> {
        let store = self.0.read().unwrap_or_else(PoisonError::into_inner);
        store.contains_sub_index(sub_index)
    }

    /// Return an the inner [`InvoiceStorage`] object wrapped in a [`RwLockReadGuard`]. This allows the
    /// caller to call [`InvoiceStorage::iter`] without encountering lifetime issues.
    pub fn lock(&self) -> RwLockReadGuard<'_, S> {
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn lowest_height(&self) -> Result<Option<u64>, S::Error> {
        let store = self.0.read().unwrap_or_else(PoisonError::into_inner);
        store.lowest_height()
    }

    pub fn is_empty(&self) -> bool {
        let store = self.0.read().unwrap_or_else(PoisonError::into_inner);
        store.is_empty()
    }

    pub fn flush(&self) -> Result<(), S::Error> {
        let store = self.0.read().unwrap_or_else(PoisonError::into_inner);
        store.flush()
    }
}

impl<S: InvoiceStorage> Clone for Store<S> {
    fn clone(&self) -> Self {
        Store(self.0.clone())
    }
}
