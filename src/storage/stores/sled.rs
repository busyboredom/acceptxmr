use thiserror::Error;

use crate::{storage::InvoiceStorage, Invoice, InvoiceId, SubIndex};

/// Sled database containing pending invoices. Note that [sled](sled) is still in beta.
pub struct Sled(sled::Tree);

impl Sled {
    /// Open a [Sled](sled) database at the specified location, and use the specified tree. Creates
    /// a new database if one does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the database could not be opened at the specified path.
    pub fn new(path: &str, tree: &str) -> Result<Sled, SledStorageError> {
        let db = sled::Config::default()
            .path(path)
            .flush_every_ms(None)
            .open()?;
        let tree = db.open_tree(tree)?;

        // Set merge operator to act as an update().
        tree.set_merge_operator(Sled::update_merge);

        Ok(Sled(tree))
    }

    fn update_merge(_key: &[u8], old_value: Option<&[u8]>, new_value: &[u8]) -> Option<Vec<u8>> {
        if old_value.is_some() {
            Some(new_value.to_vec())
        } else {
            None
        }
    }
}

impl InvoiceStorage for Sled {
    type Error = SledStorageError;
    type Iter<'a> = SledIter;

    fn insert(&mut self, invoice: Invoice) -> Result<Option<Invoice>, SledStorageError> {
        // Prepare key (invoice id).
        let invoice_id = invoice.id();
        let key = bincode::encode_to_vec(invoice_id, bincode::config::standard())?;

        // Prepare value (invoice).
        let value = bincode::encode_to_vec(invoice, bincode::config::standard())?;

        // Insert the invoice into the database.
        let maybe_old = self.0.insert(key, value)?;

        match maybe_old {
            Some(ivec) => Ok(Some(
                bincode::decode_from_slice(&ivec, bincode::config::standard())?.0,
            )),
            None => Ok(None),
        }
    }

    fn remove(&mut self, invoice_id: InvoiceId) -> Result<Option<Invoice>, SledStorageError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(invoice_id, bincode::config::standard())?;

        let old = self.0.remove(key).transpose();
        old.map(|ivec_or_err| {
            Ok(bincode::decode_from_slice(&ivec_or_err?, bincode::config::standard())?.0)
        })
        .transpose()
    }

    fn update(&mut self, invoice: Invoice) -> Result<Option<Invoice>, SledStorageError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(invoice.id(), bincode::config::standard())?;

        // Prepare values.
        let new_ivec = bincode::encode_to_vec(invoice, bincode::config::standard())?;

        // Do the insert using the merge operator configured.
        let maybe_old = self.0.merge(key, new_ivec)?;

        match maybe_old {
            Some(ivec) => Ok(Some(
                bincode::decode_from_slice(&ivec, bincode::config::standard())?.0,
            )),
            None => Ok(None),
        }
    }

    fn get(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, SledStorageError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(invoice_id, bincode::config::standard())?;

        let current = self.0.get(key).transpose();
        current
            .map(|ivec_or_err| {
                Ok(bincode::decode_from_slice(&ivec_or_err?, bincode::config::standard())?.0)
            })
            .transpose()
    }

    fn contains_sub_index(&self, sub_index: SubIndex) -> Result<bool, SledStorageError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(sub_index, bincode::config::standard())?;

        Ok(self.0.scan_prefix(key).next().is_some())
    }

    fn iter(&self) -> Self::Iter<'_> {
        SledIter(self.0.iter())
    }

    /// Flush all changes to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if flush does not succeed.
    fn flush(&self) -> Result<(), SledStorageError> {
        self.0.flush()?;
        Ok(())
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub struct SledIter(sled::Iter);

impl Iterator for SledIter {
    type Item = Result<Invoice, SledStorageError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.0.next()? {
            Ok((_, value)) => {
                let invoice_or_err =
                    bincode::decode_from_slice(&value, bincode::config::standard())
                        .map(|v| v.0)
                        .map_err(SledStorageError::Deserialize);
                Some(invoice_or_err)
            }
            Err(e) => Some(Err(SledStorageError::Database(e))),
        }
    }
}

/// An error occurring while storing or retrieving pending invoices from a `sled` database.
#[derive(Error, Debug)]
pub enum SledStorageError {
    /// An error caused by the database, or some interaction with it.
    #[error("database error: {0}")]
    Database(#[from] sled::Error),
    /// Failed to serialize an [`Invoice`].
    #[error("Serialization error: {0}")]
    Serialize(#[from] bincode::error::EncodeError),
    /// Failed to deserialize an [`Invoice`].
    #[error("Deserialization error: {0}")]
    Deserialize(#[from] bincode::error::DecodeError),
}
