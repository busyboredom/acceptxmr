/// The [`HeightStorage`] trait describes the block height storage
/// layer for `AcceptXMR`. This layer allows the payment gateway to track the
/// most recently scanned block.
pub trait HeightStorage: Send + Sync {
    /// Error type for the storage layer.
    type Error: std::error::Error + Send + 'static;

    /// Updates the payment gateway's block height, or inserts the height if
    /// there is nothing to update. Returns the old height if it existed.
    ///
    /// # Errors
    ///
    /// Returns an error if there was an underlying issue with the database.
    fn upsert(&mut self, height: u64) -> Result<Option<u64>, Self::Error>;

    /// Returns the block height of the payment gateway, if it exists.
    ///
    /// # Errors
    ///
    /// Returns an error if there was an underlying issue with the database.
    fn get(&self) -> Result<Option<u64>, Self::Error>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod test {
    use std::fmt::{Debug, Display};

    use tempfile::Builder;
    use test_case::test_case;

    use crate::storage::{
        stores::{InMemory, Sled, Sqlite},
        HeightStorage,
    };

    fn new_temp_dir() -> String {
        Builder::new()
            .prefix("temp_db_")
            .rand_bytes(16)
            .tempdir()
            .unwrap()
            .path()
            .to_str()
            .expect("failed to get temporary directory path")
            .to_string()
    }

    #[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(InMemory::new(); "in-memory")]
    #[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn upsert_and_check<S, E>(mut store: S)
    where
        S: HeightStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        store.upsert(123).unwrap();
        assert_eq!(store.get().unwrap(), Some(123));
    }

    #[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(InMemory::new(); "in-memory")]
    #[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn upsert_existing<S, E>(mut store: S)
    where
        S: HeightStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        store.upsert(123).unwrap();
        assert_eq!(store.get().unwrap(), Some(123));

        assert_eq!(store.upsert(124).unwrap(), Some(123));
        assert_eq!(store.get().unwrap(), Some(124));
    }

    #[test_case(&Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(&InMemory::new(); "in-memory")]
    #[test_case(&Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn doesnt_contain_key<S, E>(store: &S)
    where
        S: HeightStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        assert!(store.get().unwrap().is_none());
    }
}
