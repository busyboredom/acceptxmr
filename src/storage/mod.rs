//! `AcceptXMR` can store pending invoices using a storage layer of your
//! choosing. Consumers of this library can use one of the existing storage
//! layers found in [`stores`], or can implement the [`InvoiceStorage`] trait
//! themselves for a custom storage solution.

pub mod stores;

use std::{
    cmp::Ordering,
    fmt::Display,
    sync::{Arc, PoisonError, RwLock, RwLockReadGuard},
};

use crate::{Invoice, InvoiceId, SubIndex};

/// The [`InvoiceStorage`] trait describes the storage layer for pending
/// invoices. Consumers of this library can use one of the existing storage
/// layers found in [`stores`], or implement this trait themselves for a custom
/// storage solution.
pub trait InvoiceStorage: Send + Sync {
    /// Error type for the storage layer.
    type Error: Display + Send;
    /// An iterator over all invoices in storage.
    type Iter<'a>: Iterator<Item = Result<Invoice, Self::Error>>
    where
        Self: 'a;

    /// Insert invoice into storage for tracking.
    ///
    /// # Errors
    ///
    /// Returns an error if the invoice could not be inserted, or if it already
    /// exists.
    fn insert(&mut self, invoice: Invoice) -> Result<(), Self::Error>;

    /// Remove invoice from storage, returning the invoice if it existed.
    ///
    /// # Errors
    ///
    /// Returns an error if the invoice could not be removed.
    fn remove(&mut self, invoice_id: InvoiceId) -> Result<Option<Invoice>, Self::Error>;

    /// Update existing invoice in storage, returning old value if it existed.
    /// If the invoice does not already exist, does nothing.
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
    /// Returns an error if the existence of an invoice for the given subaddress
    /// could not be determined.
    fn contains_sub_index(&self, sub_index: SubIndex) -> Result<bool, Self::Error>;

    /// Returns an iterator over all invoices in storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the iterator could not be created due to an
    /// underlying issue with the storage layer.
    fn try_iter(&self) -> Result<Self::Iter<'_>, Self::Error>;

    /// Recover lowest current height of an invoice in storage. Scanning will
    /// resume from this height.
    ///
    /// # Errors
    ///
    /// Returns an error if the lowest height of an invoice could not be
    /// determined.
    fn lowest_height(&self) -> Result<Option<u64>, Self::Error> {
        self.try_iter()?
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
    ///
    /// # Errors
    ///
    /// Returns an error if there was an underlying issue with the storage
    /// layer.
    fn is_empty(&self) -> Result<bool, Self::Error> {
        Ok(self.try_iter()?.next().is_none())
    }

    /// Flush all changes to disk. This method should be manually implemented
    /// for any storage layer that does not automatically flush on write. The
    /// default implementation does nothing.
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

    pub fn insert(&self, invoice: Invoice) -> Result<(), S::Error> {
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

    /// Return an the inner [`InvoiceStorage`] object wrapped in a
    /// [`RwLockReadGuard`]. This allows the caller to call
    /// [`InvoiceStorage::iter`] without encountering lifetime issues.
    pub fn lock(&self) -> RwLockReadGuard<'_, S> {
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn lowest_height(&self) -> Result<Option<u64>, S::Error> {
        let store = self.0.read().unwrap_or_else(PoisonError::into_inner);
        store.lowest_height()
    }

    pub fn is_empty(&self) -> Result<bool, S::Error> {
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod test {
    use std::fmt::{Debug, Display};

    use tempfile::Builder;
    use test_case::test_case;

    use crate::{
        storage::{
            stores::{InMemory, Sled, Sqlite},
            InvoiceStorage,
        },
        Invoice, SubIndex,
    };

    pub fn new_temp_dir() -> String {
        Builder::new()
            .prefix("temp_db_")
            .rand_bytes(16)
            .tempdir()
            .expect("failed to generate temporary directory")
            .path()
            .to_str()
            .expect("failed to get temporary directory path")
            .to_string()
    }

    fn dummy_invoice() -> Invoice {
        Invoice::new(
            "4A1WSBQdCbUCqt3DaGfmqVFchXScF43M6c5r4B6JXT3dUwuALncU9XTEnRPmUMcB3c16kVP9Y7thFLCJ5BaMW3UmSy93w3w".to_string(),
            SubIndex::new(123, 123),
            123,
            1,
            1,
            1,
            "description".to_string(),
        )
    }

    #[test_case(Sled::new(&new_temp_dir(), "tree").unwrap())]
    #[test_case(InMemory::new())]
    #[test_case(Sqlite::new(":memory:", "invoices").unwrap())]
    fn insert_and_get<'a, S, E, I>(mut store: S)
    where
        S: InvoiceStorage<Error = E, Iter<'a> = I> + 'static,
        E: Debug + Display + Send,
        I: Iterator,
    {
        let invoice = dummy_invoice();
        store.insert(invoice.clone()).unwrap();
        assert_eq!(store.get(invoice.id()).unwrap(), Some(invoice));
    }

    #[test_case(Sled::new(&new_temp_dir(), "tree").unwrap())]
    #[test_case(InMemory::new())]
    #[test_case(Sqlite::new(":memory:", "invoices").unwrap())]
    fn insert_existing<'a, S, E, I>(mut store: S)
    where
        S: InvoiceStorage<Error = E, Iter<'a> = I> + 'static,
        E: Debug + Display + Send,
        I: Iterator,
    {
        let mut invoice = dummy_invoice();

        store.insert(invoice.clone()).unwrap();
        assert_eq!(store.get(invoice.id()).unwrap(), Some(invoice.clone()));

        invoice.description = "test".to_string();
        store
            .insert(invoice.clone())
            .expect_err("inserting existing invoice should fail");
        assert_ne!(store.get(invoice.id()).unwrap(), Some(invoice));
    }

    #[test_case(Sled::new(&new_temp_dir(), "tree").unwrap())]
    #[test_case(InMemory::new())]
    #[test_case(Sqlite::new(":memory:", "invoices").unwrap())]
    fn remove<'a, S, E, I>(mut store: S)
    where
        S: InvoiceStorage<Error = E, Iter<'a> = I> + 'static,
        E: Debug + Display + Send,
        I: Iterator,
    {
        let invoice = dummy_invoice();
        store.insert(invoice.clone()).unwrap();
        assert_eq!(store.get(invoice.id()).unwrap(), Some(invoice.clone()));

        assert_eq!(store.remove(invoice.id()).unwrap(), Some(invoice.clone()));
        assert_eq!(store.get(invoice.id()).unwrap(), None);
    }

    #[test_case(Sled::new(&new_temp_dir(), "tree").unwrap())]
    #[test_case(InMemory::new())]
    #[test_case(Sqlite::new(":memory:", "invoices").unwrap())]
    fn remove_non_existent<'a, S, E, I>(mut store: S)
    where
        S: InvoiceStorage<Error = E, Iter<'a> = I> + 'static,
        E: Debug + Display + Send,
        I: Iterator,
    {
        let invoice = dummy_invoice();
        assert_eq!(store.get(invoice.id()).unwrap(), None);

        assert_eq!(store.remove(invoice.id()).unwrap(), None);
        assert_eq!(store.get(invoice.id()).unwrap(), None);
    }

    #[test_case(Sled::new(&new_temp_dir(), "tree").unwrap())]
    #[test_case(InMemory::new())]
    #[test_case(Sqlite::new(":memory:", "invoices").unwrap())]
    fn update<'a, S, E, I>(mut store: S)
    where
        S: InvoiceStorage<Error = E, Iter<'a> = I> + 'static,
        E: Debug + Display + Send,
        I: Iterator,
    {
        let invoice = dummy_invoice();
        store.insert(invoice.clone()).unwrap();

        let mut updated_invoice = invoice.clone();
        updated_invoice.description = "test".to_string();

        assert_eq!(
            store.update(updated_invoice.clone()).unwrap(),
            Some(invoice.clone())
        );
        assert_eq!(store.get(invoice.id()).unwrap(), Some(updated_invoice));
    }

    #[test_case(Sled::new(&new_temp_dir(), "tree").unwrap())]
    #[test_case(InMemory::new())]
    #[test_case(Sqlite::new(":memory:", "invoices").unwrap())]
    fn update_empty<'a, S, E, I>(mut store: S)
    where
        S: InvoiceStorage<Error = E, Iter<'a> = I> + 'static,
        E: Debug + Display + Send,
        I: Iterator,
    {
        let invoice = dummy_invoice();
        assert_eq!(store.update(invoice.clone()).unwrap(), None);
        assert_eq!(store.get(invoice.id()).unwrap(), None);
    }

    #[test_case(&Sled::new(&new_temp_dir(), "tree").unwrap())]
    #[test_case(&InMemory::new())]
    #[test_case(&Sqlite::new(":memory:", "invoices").unwrap())]
    fn get_non_existent<'a, S, E, I>(store: &S)
    where
        S: InvoiceStorage<Error = E, Iter<'a> = I> + 'static,
        E: Debug + Display + Send,
        I: Iterator,
    {
        let invoice = dummy_invoice();
        assert_eq!(store.get(invoice.id()).unwrap(), None);
        // Try again just in case the first `get` mutated state.
        assert_eq!(store.get(invoice.id()).unwrap(), None);
    }

    #[test_case(Sled::new(&new_temp_dir(), "tree").unwrap())]
    #[test_case(InMemory::new())]
    #[test_case(Sqlite::new(":memory:", "invoices").unwrap())]
    fn contains_subindex<'a, S, E, I>(mut store: S)
    where
        S: InvoiceStorage<Error = E, Iter<'a> = I> + 'static,
        E: Debug + Display + Send,
        I: Iterator,
    {
        let invoice = dummy_invoice();
        store.insert(invoice).unwrap();

        assert!(store.contains_sub_index(SubIndex::new(123, 123)).unwrap());
    }

    #[test_case(&Sled::new(&new_temp_dir(), "tree").unwrap())]
    #[test_case(&InMemory::new())]
    #[test_case(&Sqlite::new(":memory:", "invoices").unwrap())]
    fn doesnt_contain_subindex<'a, S, E, I>(store: &S)
    where
        S: InvoiceStorage<Error = E, Iter<'a> = I> + 'static,
        E: Debug + Display + Send,
        I: Iterator,
    {
        assert!(!store.contains_sub_index(SubIndex::new(123, 123)).unwrap());
    }

    #[test_case(&mut Sled::new(&new_temp_dir(), "tree").unwrap() => ())]
    #[test_case(&mut InMemory::new() => ())]
    #[test_case(&mut Sqlite::new(":memory:", "invoices").unwrap() => ())]
    fn iterate<'a, S, E, I>(store: &'a mut S)
    where
        S: InvoiceStorage<Error = E, Iter<'a> = I>,
        E: Debug + Display + Send,
        I: Iterator<Item = Result<Invoice, E>>,
    {
        let invoice = dummy_invoice();
        store.insert(invoice.clone()).unwrap();

        let mut iter = store.try_iter().unwrap();
        assert_eq!(iter.next().transpose().unwrap(), Some(invoice));
        assert_eq!(iter.next().transpose().unwrap(), None);
    }

    #[test_case(&mut Sled::new(&new_temp_dir(), "tree").unwrap())]
    #[test_case(&mut InMemory::new())]
    #[test_case(&mut Sqlite::new(":memory:", "invoices").unwrap())]
    fn iterate_empty<'a, S, E, I>(store: &'a mut S)
    where
        S: InvoiceStorage<Error = E, Iter<'a> = I>,
        E: Debug + Display + Send,
        I: Iterator<Item = Result<Invoice, E>>,
    {
        let mut iter = store.try_iter().unwrap();
        assert_eq!(iter.next().transpose().unwrap(), None);
    }
}
