use std::error::Error;
use std::{cmp::Ordering, fmt};

use crate::subscriber::Subscriber;
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
        let key = bincode::encode_to_vec(&invoice_id, bincode::config::standard())?;

        // Prepare value (invoice).
        let value = bincode::encode_to_vec(&invoice, bincode::config::standard())?;

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
        let key = bincode::encode_to_vec(&invoice_id, bincode::config::standard())?;

        let old = self.0.remove(key).transpose();
        old.map(|ivec_or_err| {
            Ok(bincode::decode_from_slice(&ivec_or_err?, bincode::config::standard())?.0)
        })
        .transpose()
    }

    pub fn get(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, InvoiceStorageError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(&invoice_id, bincode::config::standard())?;

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

    pub fn contains_key(&self, invoice_id: InvoiceId) -> Result<bool, InvoiceStorageError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(&invoice_id, bincode::config::standard())?;

        self.0.contains_key(key).map_err(InvoiceStorageError::from)
    }

    pub fn contains_sub_index(&self, sub_index: SubIndex) -> Result<bool, InvoiceStorageError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(&sub_index, bincode::config::standard())?;

        Ok(self.0.scan_prefix(key).next().is_some())
    }

    pub fn update(&self, invoice_id: InvoiceId, new: &Invoice) -> Result<Invoice, AcceptXmrError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(&invoice_id, bincode::config::standard())
            .map_err(InvoiceStorageError::from)?;

        // Prepare values.
        let new_ivec = bincode::encode_to_vec(&new, bincode::config::standard())
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

    pub fn subscribe(
        &self,
        invoice_id: InvoiceId,
    ) -> Result<Option<Subscriber>, InvoiceStorageError> {
        let prefix = bincode::encode_to_vec(&invoice_id, bincode::config::standard())?;
        let sled_subscriber = self.0.watch_prefix(prefix);
        if self.contains_key(invoice_id)? {
            Ok(Some(Subscriber::new(sled_subscriber)))
        } else {
            Ok(None)
        }
    }

    pub fn subscribe_all(&self) -> Subscriber {
        let sled_subscriber = self.0.watch_prefix(vec![]);
        Subscriber::new(sled_subscriber)
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
    Update(InvoiceId),
    /// Failed to serialize an [`Invoice`].
    Serialize(bincode::error::EncodeError),
    /// Failed to deserialize an [`Invoice`].
    Deserialize(bincode::error::DecodeError),
}

impl From<sled::Error> for InvoiceStorageError {
    fn from(e: sled::Error) -> Self {
        Self::Database(e)
    }
}

impl From<bincode::error::EncodeError> for InvoiceStorageError {
    fn from(e: bincode::error::EncodeError) -> Self {
        Self::Serialize(e)
    }
}

impl From<bincode::error::DecodeError> for InvoiceStorageError {
    fn from(e: bincode::error::DecodeError) -> Self {
        Self::Deserialize(e)
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
            InvoiceStorageError::Serialize(bincode_error) => {
                write!(f, "Serialization error: {}", bincode_error)
            }
            InvoiceStorageError::Deserialize(bincode_error) => {
                write!(f, "Deserialization error: {}", bincode_error)
            }
        }
    }
}

impl Error for InvoiceStorageError {}
