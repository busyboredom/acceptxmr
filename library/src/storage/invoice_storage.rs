use crate::{Invoice, InvoiceId, SubIndex};

/// The [`InvoiceStorage`] trait describes the invoice storage layer for
/// `AcceptXMR`.
pub trait InvoiceStorage: Send + Sync {
    /// Error type for the storage layer.
    type Error: std::error::Error + Send + 'static;

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

    /// Retrieve all currently-tracked invoice ids from storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the invoice could not read.
    fn get_ids(&self) -> Result<Vec<InvoiceId>, Self::Error>;

    /// Returns whether an invoice for the given subaddress exists in storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the existence of an invoice for the given subaddress
    /// could not be determined.
    fn contains_sub_index(&self, sub_index: SubIndex) -> Result<bool, Self::Error>;

    /// Iterates over all invoices in storage, executing the supplied closure on
    /// each.
    ///
    /// # Errors
    ///
    /// Stops iterating and returns an error if the supplied closure returns an
    /// error.
    fn try_for_each<F>(&self, f: F) -> Result<(), Self::Error>
    where
        F: FnMut(Result<Invoice, Self::Error>) -> Result<(), Self::Error>;

    /// Returns `true` if there are no invoices in storage.
    ///
    /// # Errors
    ///
    /// Returns an error if there was an underlying issue with the storage
    /// layer.
    fn is_empty(&self) -> Result<bool, Self::Error>;

    /// Find lowest current height of an invoice in storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the lowest height of an invoice could not be
    /// determined.
    fn lowest_height(&self) -> Result<Option<u64>, Self::Error> {
        let mut lowest = None;
        self.try_for_each(|invoice_or_err| {
            let current_height = invoice_or_err?.current_height();
            match lowest {
                Some(l) if l > current_height => {
                    lowest = Some(current_height);
                }
                None => {
                    lowest = Some(current_height);
                }
                Some(_) => {}
            }
            Ok(())
        })?;

        Ok(lowest)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod test {
    use std::fmt::{Debug, Display};

    use test_case::test_case;
    use testing_utils::new_temp_dir;

    use crate::{
        storage::{
            stores::{InMemory, Sled, Sqlite},
            InvoiceStorage,
        },
        Invoice, SubIndex,
    };

    fn dummy_invoice() -> Invoice {
        Invoice::new(
            "4a1wsbqdcbucqt3dagfmqvfchxscf43m6c5r4b6jxt3duwualncu9xtenrpmumcb3c16kvp9y7thflcj5bamw3umsy93w3w".to_string(),
            SubIndex::new(123, 123),
            123,
            1,
            1,
            1,
            "description".to_string(),
        )
    }

    fn dummy_invoice_2() -> Invoice {
        Invoice::new(
            "4A1WSBQdCbUCqt3DaGfmqVFchXScF43M6c5r4B6JXT3dUwuALncU9XTEnRPmUMcB3c16kVP9Y7thFLCJ5BaMW3UmSy93w3w".to_string(),
            SubIndex::new(321, 321),
            321,
           2,
            2,
            2,
            "description_2".to_string(),
        )
    }

    #[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(InMemory::new(); "in-memory")]
    #[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn insert_and_get<S, E>(mut store: S)
    where
        S: InvoiceStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        let invoice = dummy_invoice();
        store.insert(invoice.clone()).unwrap();
        assert_eq!(store.get(invoice.id()).unwrap(), Some(invoice));
    }

    #[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(InMemory::new(); "in-memory")]
    #[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn insert_existing<S, E>(mut store: S)
    where
        S: InvoiceStorage<Error = E> + 'static,
        E: Debug + Display + Send,
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

    #[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(InMemory::new(); "in-memory")]
    #[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn remove<S, E>(mut store: S)
    where
        S: InvoiceStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        let invoice = dummy_invoice();
        store.insert(invoice.clone()).unwrap();
        assert_eq!(store.get(invoice.id()).unwrap(), Some(invoice.clone()));

        assert_eq!(store.remove(invoice.id()).unwrap(), Some(invoice.clone()));
        assert_eq!(store.get(invoice.id()).unwrap(), None);
    }

    #[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(InMemory::new(); "in-memory")]
    #[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn remove_non_existent<S, E>(mut store: S)
    where
        S: InvoiceStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        let invoice = dummy_invoice();
        assert_eq!(store.get(invoice.id()).unwrap(), None);

        assert_eq!(store.remove(invoice.id()).unwrap(), None);
        assert_eq!(store.get(invoice.id()).unwrap(), None);
    }

    #[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(InMemory::new(); "in-memory")]
    #[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn update<S, E>(mut store: S)
    where
        S: InvoiceStorage<Error = E> + 'static,
        E: Debug + Display + Send,
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

    #[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(InMemory::new(); "in-memory")]
    #[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn update_empty<S, E>(mut store: S)
    where
        S: InvoiceStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        let invoice = dummy_invoice();
        assert_eq!(store.update(invoice.clone()).unwrap(), None);
        assert_eq!(store.get(invoice.id()).unwrap(), None);
    }

    #[test_case(&Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(&InMemory::new(); "in-memory")]
    #[test_case(&Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn get_non_existent<S, E>(store: &S)
    where
        S: InvoiceStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        let invoice = dummy_invoice();
        assert_eq!(store.get(invoice.id()).unwrap(), None);
        // Try again just in case the first `get` mutated state.
        assert_eq!(store.get(invoice.id()).unwrap(), None);
    }

    #[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(InMemory::new(); "in-memory")]
    #[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn get_ids<S, E>(mut store: S)
    where
        S: InvoiceStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        assert_eq!(store.get_ids().unwrap(), Vec::new());

        let invoice_1 = dummy_invoice();
        store.insert(invoice_1.clone()).unwrap();

        assert_eq!(store.get_ids().unwrap(), vec![invoice_1.id()]);

        let invoice_2 = dummy_invoice_2();
        store.insert(invoice_2.clone()).unwrap();

        let mut expected_ids = vec![invoice_1.id(), invoice_2.id()];
        expected_ids.sort();
        let mut actual_ids = store.get_ids().unwrap();
        actual_ids.sort();

        assert_eq!(expected_ids, actual_ids);
    }

    #[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(InMemory::new(); "in-memory")]
    #[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn contains_subindex<S, E>(mut store: S)
    where
        S: InvoiceStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        let invoice = dummy_invoice();
        store.insert(invoice).unwrap();

        assert!(store.contains_sub_index(SubIndex::new(123, 123)).unwrap());
    }

    #[test_case(&Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(&InMemory::new(); "in-memory")]
    #[test_case(&Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn doesnt_contain_subindex<S, E>(store: &S)
    where
        S: InvoiceStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        assert!(!store.contains_sub_index(SubIndex::new(123, 123)).unwrap());
    }

    #[test_case(&mut Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(&mut InMemory::new(); "in-memory")]
    #[test_case(&mut Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn for_each<S, E>(store: &mut S)
    where
        S: InvoiceStorage<Error = E>,
        E: Debug + Display + Send,
    {
        let invoice = dummy_invoice();
        store.insert(invoice.clone()).unwrap();
        let mut count = 0;

        store
            .try_for_each(|invoice_or_err| {
                assert_eq!(invoice_or_err.unwrap(), invoice);
                count += 1;
                Ok(())
            })
            .unwrap();

        assert_eq!(count, 1);
    }

    #[test_case(&mut Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(&mut InMemory::new(); "in-memory")]
    #[test_case(&mut Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn for_each_empty<S, E>(store: &mut S)
    where
        S: InvoiceStorage<Error = E>,
        E: Debug + Display + Send,
    {
        let invoice = dummy_invoice();
        let mut count = 0;

        store
            .try_for_each(|invoice_or_err| {
                assert_eq!(invoice_or_err.unwrap(), invoice);
                count += 1;
                Ok(())
            })
            .unwrap();

        assert_eq!(count, 0);
    }

    #[test_case(&mut Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(&mut InMemory::new(); "in-memory")]
    #[test_case(&mut Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn is_empty<S, E>(store: &mut S)
    where
        S: InvoiceStorage<Error = E>,
        E: Debug + Display + Send,
    {
        assert!(store.is_empty().unwrap());

        let invoice = dummy_invoice();
        store.insert(invoice.clone()).unwrap();
        assert!(!store.is_empty().unwrap());

        store.remove(invoice.id()).unwrap();
        assert!(store.is_empty().unwrap());
    }

    #[test_case(&mut Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(&mut InMemory::new(); "in-memory")]
    #[test_case(&mut Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn lowest_height<S, E>(store: &mut S)
    where
        S: InvoiceStorage<Error = E>,
        E: Debug + Display + Send,
    {
        assert_eq!(store.lowest_height().unwrap(), None);

        let invoice = dummy_invoice();
        store.insert(invoice.clone()).unwrap();

        assert_eq!(store.lowest_height().unwrap(), Some(0));
    }
}
