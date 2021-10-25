use std::error::Error;
use std::{cmp::Ordering, fmt};

use crate::{AcceptXmrError, Invoice, SubIndex, Subscriber};

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
        // Prepare key (subaddress index).
        let key = [
            invoice.index.major.to_be_bytes(),
            invoice.index.minor.to_be_bytes(),
        ]
        .concat();

        // Prepare value (invoice).
        let value = bincode::serialize(&invoice)?;

        // Insert the invoice into the database.
        let old = self.0.insert(key, value)?;

        if let Some(old_value) = old {
            Ok(Some(bincode::deserialize(&old_value)?))
        } else {
            Ok(None)
        }
    }

    pub fn remove(&self, sub_index: SubIndex) -> Result<Option<Invoice>, InvoiceStorageError> {
        // Prepare key (subaddress index).
        let key = [sub_index.major.to_be_bytes(), sub_index.minor.to_be_bytes()].concat();

        let old = self.0.remove(key).transpose();
        old.map(|ivec_or_err| Ok(bincode::deserialize(&ivec_or_err?)?))
            .transpose()
    }

    pub fn iter(
        &self,
    ) -> impl DoubleEndedIterator<Item = Result<Invoice, InvoiceStorageError>> + Send + Sync {
        // Convert iterator of Result<IVec> to Result<Invoice>.
        self.0.iter().values().flat_map(|r| {
            r.map(|ivec| bincode::deserialize(&ivec).map_err(InvoiceStorageError::from))
                .map_err(InvoiceStorageError::from)
        })
    }

    pub fn contains_key(&self, sub_index: SubIndex) -> Result<bool, InvoiceStorageError> {
        // Prepare key (subaddress index).
        let key = [sub_index.major.to_be_bytes(), sub_index.minor.to_be_bytes()].concat();

        self.0.contains_key(key).map_err(InvoiceStorageError::from)
    }

    pub fn update(&self, sub_index: SubIndex, new: &Invoice) -> Result<Invoice, AcceptXmrError> {
        // Prepare key (subaddress index).
        let key = [sub_index.major.to_be_bytes(), sub_index.minor.to_be_bytes()].concat();

        // Prepare values.
        let new_ivec = bincode::serialize(&new).map_err(InvoiceStorageError::from)?;

        // Do the update using the merge operator configured when InvoiceDb is constructed.
        let maybe_old = self
            .0
            .merge(key, new_ivec)
            .map_err(InvoiceStorageError::from)?;
        match maybe_old {
            Some(ivec) => Ok(bincode::deserialize(&ivec).map_err(InvoiceStorageError::from)?),
            None => Err(AcceptXmrError::from(InvoiceStorageError::Update(sub_index))),
        }
    }

    pub fn subscribe(&self, sub_index: SubIndex) -> Subscriber {
        let mut prefix = Vec::new();
        // If asked to subscribe to the primary address index, watch everything. Otherwise, watch that specific index.
        if sub_index != SubIndex::new(0, 0) {
            prefix = [sub_index.major.to_be_bytes(), sub_index.minor.to_be_bytes()].concat();
        }
        let sled_subscriber = self.0.watch_prefix(prefix);
        Subscriber::new(sled_subscriber)
    }

    pub fn flush(&self) {
        self.0
            .flush()
            .expect("failed to flush invoice updates to invoices database");
    }

    pub fn clone(&self) -> InvoicesDb {
        InvoicesDb(self.0.clone())
    }

    /// Recover lowest height. This performs a full O(n) scan of the database. Returns None if the
    /// database is empty.
    pub fn lowest_height(&self) -> Result<Option<u64>, InvoiceStorageError> {
        self.iter()
            .min_by(|invoice_1, invoice_2| {
                // If there is an error, we want it returned.
                if invoice_1.is_err() {
                    Ordering::Greater
                } else if invoice_2.is_err() {
                    Ordering::Less
                } else {
                    // Otherwise, return the one with the lower height.
                    invoice_1
                        .as_ref()
                        .unwrap()
                        .current_height
                        .cmp(&invoice_2.as_ref().unwrap().current_height)
                }
            })
            .transpose()
            .map(|maybe_invoice| maybe_invoice.map(|invoice| invoice.current_height))
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
#[derive(Debug)]
pub enum InvoiceStorageError {
    /// An error caused by the database, or some interaction with it.
    Database(sled::Error),
    /// A [`Invoice`] in the database can not be updated, because the
    /// `Invoice` does not exist.
    Update(SubIndex),
    /// Failed to (de)serialize a [`Invoice`].
    Serialization(bincode::Error),
}

impl From<sled::Error> for InvoiceStorageError {
    fn from(e: sled::Error) -> Self {
        Self::Database(e)
    }
}

impl From<bincode::Error> for InvoiceStorageError {
    fn from(e: bincode::Error) -> Self {
        Self::Serialization(e)
    }
}

impl fmt::Display for InvoiceStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvoiceStorageError::Database(sled_error) => {
                write!(f, "database error: {}", sled_error)
            }
            InvoiceStorageError::Update(key) => {
                write!(f, "no value with key {} to update", key)
            }
            InvoiceStorageError::Serialization(bincode_error) => {
                write!(f, "(de)serialization error: {}", bincode_error)
            }
        }
    }
}

impl Error for InvoiceStorageError {}
