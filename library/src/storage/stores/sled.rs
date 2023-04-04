use sled::{
    transaction::{ConflictableTransactionError, TransactionError},
    IVec,
};
use thiserror::Error;

use crate::{
    storage::{HeightStorage, InvoiceStorage, OutputId, OutputKeyStorage, OutputPubKey, Storage},
    Invoice, InvoiceId, SubIndex,
};

/// Sled database. Note that [sled](sled) is still in beta.
pub struct Sled {
    invoices: sled::Tree,
    output_keys: sled::Tree,
    height: sled::Tree,
}

impl Sled {
    /// Open a [Sled](sled) database at the specified location, and use the
    /// specified tree. Creates a new database if one does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the database could not be opened at the specified
    /// path.
    pub fn new(
        path: &str,
        invoice_tree: &str,
        output_key_tree: &str,
        height_tree: &str,
    ) -> Result<Sled, SledStorageError> {
        let db = sled::Config::default()
            .path(path)
            .flush_every_ms(None)
            .open()
            .map_err(DatabaseError::from)?;
        let invoices = db.open_tree(invoice_tree).map_err(DatabaseError::from)?;
        let output_keys = db.open_tree(output_key_tree).map_err(DatabaseError::from)?;
        let height = db.open_tree(height_tree).map_err(DatabaseError::from)?;

        // Set merge operator to act as an update().
        invoices.set_merge_operator(Sled::update_merge);

        Ok(Sled {
            invoices,
            output_keys,
            height,
        })
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

    fn insert(&mut self, invoice: Invoice) -> Result<(), SledStorageError> {
        // Prepare key (invoice id).
        let invoice_id = invoice.id();
        let key = bincode::encode_to_vec(invoice_id, bincode::config::standard())?;

        // Prepare value (invoice).
        let value = bincode::encode_to_vec(invoice, bincode::config::standard())?;

        // Insert the invoice into the database.
        match self
            .invoices
            .compare_and_swap(key, None::<IVec>, Some(value))
            .map_err(DatabaseError::from)?
        {
            Ok(()) => Ok(()),
            Err(_) => Err(SledStorageError::DuplicateInvoiceId),
        }
    }

    fn remove(&mut self, invoice_id: InvoiceId) -> Result<Option<Invoice>, SledStorageError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(invoice_id, bincode::config::standard())?;

        let old = self.invoices.remove(key).transpose();
        old.map(|ivec_or_err| {
            Ok(bincode::decode_from_slice(
                &ivec_or_err.map_err(DatabaseError::from)?,
                bincode::config::standard(),
            )?
            .0)
        })
        .transpose()
    }

    fn update(&mut self, invoice: Invoice) -> Result<Option<Invoice>, SledStorageError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(invoice.id(), bincode::config::standard())?;

        // Prepare values.
        let new_ivec = bincode::encode_to_vec(invoice, bincode::config::standard())?;

        // Do the update using the merge operator configured.
        let maybe_old = self
            .invoices
            .fetch_and_update(key, move |old| {
                if old.is_some() {
                    // Clone is necessary because the closure may be called multiple times.
                    Some(new_ivec.clone())
                } else {
                    None
                }
            })
            .map_err(DatabaseError::from)?;

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

        let current = self.invoices.get(key).transpose();
        current
            .map(|ivec_or_err| {
                Ok(bincode::decode_from_slice(
                    &ivec_or_err.map_err(DatabaseError::from)?,
                    bincode::config::standard(),
                )?
                .0)
            })
            .transpose()
    }

    fn contains_sub_index(&self, sub_index: SubIndex) -> Result<bool, SledStorageError> {
        // Prepare key (invoice id).
        let key = bincode::encode_to_vec(sub_index, bincode::config::standard())?;

        Ok(self.invoices.scan_prefix(key).next().is_some())
    }

    fn try_iter(&self) -> Result<Self::Iter<'_>, SledStorageError> {
        Ok(SledIter(self.invoices.iter()))
    }

    fn is_empty(&self) -> Result<bool, SledStorageError> {
        Ok(self.invoices.is_empty())
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
            Err(e) => Some(Err(SledStorageError::Database(e.into()))),
        }
    }
}

impl OutputKeyStorage for Sled {
    type Error = SledStorageError;

    fn insert(&mut self, key: OutputPubKey, output_id: OutputId) -> Result<(), Self::Error> {
        let result = self.output_keys.transaction(move |tx| {
            let value =
                bincode::encode_to_vec(output_id, bincode::config::standard()).map_err(|e| {
                    ConflictableTransactionError::Abort(Box::new(SledStorageError::Serialize(e)))
                })?;
            match tx.insert(&key.0, value) {
                Ok(None) => Ok(()),
                Ok(Some(_)) => Err(ConflictableTransactionError::Abort(Box::new(
                    SledStorageError::DuplicateOutputKey,
                ))),
                Err(e) => Err(e)?,
            }
        });

        Ok(result.map_err(DatabaseError::from)?)
    }

    fn get(&self, key: OutputPubKey) -> Result<Option<OutputId>, Self::Error> {
        let current = self.output_keys.get(key).transpose();
        current
            .map(|ivec_or_err| {
                Ok(bincode::decode_from_slice(
                    &ivec_or_err.map_err(DatabaseError::from)?,
                    bincode::config::standard(),
                )?
                .0)
            })
            .transpose()
    }
}

impl HeightStorage for Sled {
    type Error = SledStorageError;

    fn upsert(&mut self, height: u64) -> Result<Option<u64>, Self::Error> {
        let encoded_height = bincode::encode_to_vec(height, bincode::config::standard())?;

        let maybe_ivec = self
            .height
            .insert("height", encoded_height)
            .map_err(DatabaseError::from)?;
        let old_height = maybe_ivec
            .map(|ivec| bincode::decode_from_slice(&ivec, bincode::config::standard()))
            .transpose()?
            .map(|(h, _)| h);

        Ok(old_height)
    }

    fn get(&self) -> Result<Option<u64>, Self::Error> {
        let maybe_ivec = self.height.get("height").map_err(DatabaseError::from)?;
        let height = maybe_ivec
            .map(|ivec| bincode::decode_from_slice(&ivec, bincode::config::standard()))
            .transpose()?
            .map(|(h, _)| h);

        Ok(height)
    }
}

impl Storage for Sled {
    type Error = SledStorageError;

    /// Flush all changes to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if flush does not succeed.
    fn flush(&self) -> Result<(), SledStorageError> {
        self.invoices.flush().map_err(DatabaseError::from)?;
        self.output_keys.flush().map_err(DatabaseError::from)?;
        self.height.flush().map_err(DatabaseError::from)?;
        Ok(())
    }
}

/// An error occurring while storing or retrieving values from a
/// `sled` database.
#[derive(Error, Debug)]
pub enum SledStorageError {
    /// An error caused by the database, or some interaction with it.
    #[error("database error: {0}")]
    Database(#[from] DatabaseError),
    /// Failed to insert an [`Invoice`] because one with the same ID already
    /// exists.
    #[error("duplicate invoice ID")]
    DuplicateInvoiceId,
    /// Failed to insert an [`OutputPubKey`] because an identical one already
    /// exists.
    #[error("duplicate output public key")]
    DuplicateOutputKey,
    /// Failed to serialize an [`Invoice`] or [`OutputPubKey`].
    #[error("serialization error: {0}")]
    Serialize(#[from] bincode::error::EncodeError),
    /// Failed to deserialize an [`Invoice`] or [`OutputPubKey`].
    #[error("deserialization error: {0}")]
    Deserialize(#[from] bincode::error::DecodeError),
}

/// An error occurring while storing or retrieving values from a
/// `sled` database.
#[derive(Error, Debug)]
pub enum DatabaseError {
    /// An error caused by the database, or some interaction with it.
    #[error("internal error: {0}")]
    General(#[from] sled::Error),
    /// An error encountered within a transaction.
    #[error("transaction error: {0}")]
    Transaction(#[from] TransactionError<Box<SledStorageError>>),
}
