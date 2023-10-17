#[cfg(feature = "bincode")]
use bincode::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The [`OutputKeyStorage`] trait describes the output public key storage layer
/// for `AcceptXMR`. This layer is necessary for protection against the [burning
/// bug](https://www.getmonero.org/2018/09/25/a-post-mortum-of-the-burning-bug.html).
pub trait OutputKeyStorage: Send + Sync {
    /// Error type for the storage layer.
    type Error: std::error::Error + Send + 'static;

    /// Insert an output's public key into storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the key could not be inserted, or if it already
    /// exists.
    fn insert(&mut self, key: OutputPubKey, output_id: OutputId) -> Result<(), Self::Error>;

    /// Returns the output ID associated with the given key, if it exists.
    ///
    /// # Errors
    ///
    /// Returns an error if there was an underlying issue with the database.
    fn get(&self, key: OutputPubKey) -> Result<Option<OutputId>, Self::Error>;
}

/// An output's public key.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "bincode", derive(Encode, Decode))]
pub struct OutputPubKey(pub [u8; 32]);

impl AsRef<[u8]> for OutputPubKey {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

/// A means of referring to a given output. Consists of a transaction hash and
/// the output's index in the transaction.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "bincode", derive(Encode, Decode))]
pub struct OutputId {
    /// Hash of the transaction the output is part of.
    pub tx_hash: [u8; 32],
    /// Index of the output in the transaction.
    pub index: u8,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod test {
    use std::fmt::{Debug, Display};

    use tempfile::Builder;
    use test_case::test_case;

    use crate::storage::{
        stores::{InMemory, Sled, Sqlite},
        OutputId, OutputKeyStorage, OutputPubKey,
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

    fn dummy_key() -> OutputPubKey {
        OutputPubKey([0; 32])
    }

    fn dummy_id() -> OutputId {
        OutputId {
            tx_hash: [0; 32],
            index: 13,
        }
    }

    #[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(InMemory::new(); "in-memory")]
    #[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn insert_and_check<S, E>(mut store: S)
    where
        S: OutputKeyStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        let key = dummy_key();
        let output_id = dummy_id();
        store.insert(key, output_id).unwrap();
        assert_eq!(store.get(key).unwrap(), Some(output_id));
    }

    #[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(InMemory::new(); "in-memory")]
    #[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn insert_existing<S, E>(mut store: S)
    where
        S: OutputKeyStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        let key = dummy_key();
        let output_id = dummy_id();

        store.insert(key, output_id).unwrap();
        assert_eq!(store.get(key).unwrap(), Some(output_id));

        store
            .insert(key, output_id)
            .expect_err("inserting existing key should fail");
        // Check that the key is still present.
        assert_eq!(store.get(key).unwrap(), Some(output_id));
    }

    #[test_case(Sled::new(&new_temp_dir(), "invoices", "output keys", "height").unwrap(); "sled")]
    #[test_case(InMemory::new(); "in-memory")]
    #[test_case(Sqlite::new(":memory:", "invoices", "output keys", "height").unwrap(); "sqlite")]
    fn doesnt_contain_key<S, E>(mut store: S)
    where
        S: OutputKeyStorage<Error = E> + 'static,
        E: Debug + Display + Send,
    {
        let key = OutputPubKey([1; 32]);
        let output_id = dummy_id();

        store.insert(key, output_id).unwrap();

        assert!(store.get(dummy_key()).unwrap().is_none());
    }
}
