use std::cmp::Ordering;

use thiserror::Error;

use crate::{AcceptXmrError, Invoice, InvoiceId, SubIndex};

/// Database containing pending invoices.
pub(crate) struct InvoicesDb(sled::Tree);

impl InvoicesDb {
    pub fn new(path: &str) -> Result<InvoicesDb, InvoiceStorageError> {
        let db = sled::Config::default()
            .path(path)
            .flush_every_ms(None)
            .open()?;
        let tree = db.open_tree(b"pending invoices")?;

        // Set merge operator to act as an update().
        tree.set_merge_operator(InvoicesDb::update_merge);

        Ok(InvoicesDb(tree))
    }

    pub fn insert(&self, invoice: &Invoice) -> Result<Option<Invoice>, InvoiceStorageError> {
        // Prepare key (invoice id).
        let invoice_id = invoice.id();
        let key = bincode::encode_to_vec(invoice_id, bincode::config::standard())?;

        // Prepare value (invoice).
        let value = bincode::encode_to_vec(invoice, bincode::config::standard())?;

        // Insert the invoice into the database.
        let old = self.0.insert(key, value)?;

        if let Some(old_value) = old {
            Ok(Some(
                bincode::decode_from_slice(&old_value, bincode::config::standard())?.0,
            ))
        } else {
            Ok(None)
        }
    }

    pub fn remove(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, InvoiceStorageError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(invoice_id, bincode::config::standard())?;

        let old = self.0.remove(key).transpose();
        old.map(|ivec_or_err| {
            Ok(bincode::decode_from_slice(&ivec_or_err?, bincode::config::standard())?.0)
        })
        .transpose()
    }

    pub fn get(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, InvoiceStorageError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(invoice_id, bincode::config::standard())?;

        let current = self.0.get(key).transpose();
        current
            .map(|ivec_or_err| {
                Ok(bincode::decode_from_slice(&ivec_or_err?, bincode::config::standard())?.0)
            })
            .transpose()
    }

    pub fn iter(
        &self,
    ) -> impl DoubleEndedIterator<Item = Result<Invoice, InvoiceStorageError>> + Send + Sync {
        // Convert iterator of Result<IVec> to Result<Invoice>.
        self.0.iter().values().flat_map(|r| {
            r.map(|ivec| {
                bincode::decode_from_slice(&ivec, bincode::config::standard())
                    .map_err(InvoiceStorageError::from)
                    .map(|tup| tup.0)
            })
            .map_err(InvoiceStorageError::from)
        })
    }

    pub fn contains_sub_index(&self, sub_index: SubIndex) -> Result<bool, InvoiceStorageError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(sub_index, bincode::config::standard())?;

        Ok(self.0.scan_prefix(key).next().is_some())
    }

    pub fn update(&self, invoice_id: InvoiceId, new: &Invoice) -> Result<Invoice, AcceptXmrError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(invoice_id, bincode::config::standard())
            .map_err(InvoiceStorageError::from)?;

        // Prepare values.
        let new_ivec = bincode::encode_to_vec(new, bincode::config::standard())
            .map_err(InvoiceStorageError::from)?;

        // Do the update using the merge operator configured when InvoiceDb is constructed.
        let maybe_old = self
            .0
            .merge(key, new_ivec)
            .map_err(InvoiceStorageError::from)?;
        match maybe_old {
            Some(ivec) => Ok(
                bincode::decode_from_slice(&ivec, bincode::config::standard())
                    .map_err(InvoiceStorageError::from)?
                    .0,
            ),
            None => Err(AcceptXmrError::from(InvoiceStorageError::Update(
                invoice_id,
            ))),
        }
    }

    pub fn flush(&self) -> Result<(), InvoiceStorageError> {
        self.0.flush()?;
        Ok(())
    }

    pub fn clone(&self) -> InvoicesDb {
        InvoicesDb(self.0.clone())
    }

    /// Recover lowest height. This performs a full O(n) scan of the database. Returns None if the
    /// database is empty.
    pub fn lowest_height(&self) -> Result<Option<u64>, InvoiceStorageError> {
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

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn update_merge(_key: &[u8], old_value: Option<&[u8]>, new_value: &[u8]) -> Option<Vec<u8>> {
        if old_value.is_some() {
            Some(new_value.to_vec())
        } else {
            None
        }
    }
}

/// An error occurring while storing or retrieving pending invoices.
#[derive(Error, Debug)]
pub enum InvoiceStorageError {
    /// An error caused by the database, or some interaction with it.
    #[error("database error: {0}")]
    Database(#[from] sled::Error),
    /// A [`Invoice`] in the database can not be updated, because the
    /// `Invoice` does not exist.
    #[error("no value with key {0} to update")]
    Update(InvoiceId),
    /// Failed to serialize an [`Invoice`].
    #[error("Serialization error: {0}")]
    Serialize(#[from] bincode::error::EncodeError),
    /// Failed to deserialize an [`Invoice`].
    #[error("Deserialization error: {0}")]
    Deserialize(#[from] bincode::error::DecodeError),
}
