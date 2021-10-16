use std::error::Error;
use std::{cmp::Ordering, fmt};

use crate::{AcceptXmrError, Payment, SubIndex, Subscriber};

/// Database containing pending payments.
pub(crate) struct PaymentsDb(sled::Tree);

impl PaymentsDb {
    pub fn new(path: &str) -> Result<PaymentsDb, PaymentStorageError> {
        let db = sled::Config::default()
            .path(path)
            .flush_every_ms(None)
            .open()?;
        let tree = db.open_tree(b"pending payments")?;

        // Set merge operator to act as an update().

        tree.set_merge_operator(PaymentsDb::update_merge);

        Ok(PaymentsDb(tree))
    }

    pub fn insert(&self, payment: &Payment) -> Result<Option<Payment>, PaymentStorageError> {
        // Prepare key (subaddress index).
        let key = [
            payment.index.major.to_be_bytes(),
            payment.index.minor.to_be_bytes(),
        ]
        .concat();

        // Prepare value (payment).
        let value = bincode::serialize(&payment)?;

        // Insert the payment into the database.
        let old = self.0.insert(key, value)?;

        if let Some(old_value) = old {
            Ok(Some(bincode::deserialize(&old_value)?))
        } else {
            Ok(None)
        }
    }

    pub fn remove(&self, sub_index: SubIndex) -> Result<Option<Payment>, PaymentStorageError> {
        // Prepare key (subaddress index).
        let key = [sub_index.major.to_be_bytes(), sub_index.minor.to_be_bytes()].concat();

        let old = self.0.remove(key).transpose();
        old.map(|ivec_or_err| Ok(bincode::deserialize(&ivec_or_err?)?))
            .transpose()
    }

    pub fn iter(
        &self,
    ) -> impl DoubleEndedIterator<Item = Result<Payment, PaymentStorageError>> + Send + Sync {
        // Convert iterator of Result<IVec> to Result<Payment>.
        self.0.iter().values().flat_map(|r| {
            r.map(|ivec| bincode::deserialize(&ivec).map_err(PaymentStorageError::from))
                .map_err(PaymentStorageError::from)
        })
    }

    pub fn contains_key(&self, sub_index: SubIndex) -> Result<bool, PaymentStorageError> {
        // Prepare key (subaddress index).
        let key = [sub_index.major.to_be_bytes(), sub_index.minor.to_be_bytes()].concat();

        self.0.contains_key(key).map_err(PaymentStorageError::from)
    }

    pub fn update(&self, sub_index: SubIndex, new: &Payment) -> Result<Payment, AcceptXmrError> {
        // Prepare key (subaddress index).
        let key = [sub_index.major.to_be_bytes(), sub_index.minor.to_be_bytes()].concat();

        // Prepare values.
        let new_ivec = bincode::serialize(&new).map_err(PaymentStorageError::from)?;

        // Do the update using the merge operator configured when PaymentDb is constructed.
        let maybe_old = self
            .0
            .merge(key, new_ivec)
            .map_err(PaymentStorageError::from)?;
        match maybe_old {
            Some(ivec) => Ok(bincode::deserialize(&ivec).map_err(PaymentStorageError::from)?),
            None => Err(AcceptXmrError::from(PaymentStorageError::Update(sub_index))),
        }
    }

    pub fn watch_payment(&self, sub_index: SubIndex) -> Subscriber {
        let mut prefix = Vec::new();
        // If asked to watch the primary address index, watch everything. Otherwise, watch that specific index.
        if sub_index != SubIndex::new(0, 0) {
            prefix = [sub_index.major.to_be_bytes(), sub_index.minor.to_be_bytes()].concat();
        }
        let sled_subscriber = self.0.watch_prefix(prefix);
        Subscriber::new(sled_subscriber)
    }

    pub fn flush(&self) {
        self.0
            .flush()
            .expect("failed to flush payment updates to payments database");
    }

    pub fn clone(&self) -> PaymentsDb {
        PaymentsDb(self.0.clone())
    }

    /// Recover lowest height. This performs a full O(n) scan of the database. Returns None if the
    /// database is empty.
    pub fn lowest_height(&self) -> Result<Option<u64>, PaymentStorageError> {
        self.iter()
            .min_by(|payment_1, payment_2| {
                // If there is an error, we want it returned.
                if payment_1.is_err() {
                    Ordering::Greater
                } else if payment_2.is_err() {
                    Ordering::Less
                } else {
                    // Otherwise, return the one with the lower height.
                    payment_1
                        .as_ref()
                        .unwrap()
                        .current_height
                        .cmp(&payment_2.as_ref().unwrap().current_height)
                }
            })
            .transpose()
            .map(|maybe_payment| maybe_payment.map(|payment| payment.current_height))
    }

    fn update_merge(_key: &[u8], old_value: Option<&[u8]>, new_value: &[u8]) -> Option<Vec<u8>> {
        if old_value.is_some() {
            Some(new_value.to_vec())
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub enum PaymentStorageError {
    Database(sled::Error),
    Update(SubIndex),
    Serialization(bincode::Error),
}

impl From<sled::Error> for PaymentStorageError {
    fn from(e: sled::Error) -> Self {
        Self::Database(e)
    }
}

impl From<bincode::Error> for PaymentStorageError {
    fn from(e: bincode::Error) -> Self {
        Self::Serialization(e)
    }
}

impl fmt::Display for PaymentStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaymentStorageError::Database(sled_error) => {
                write!(f, "database error: {}", sled_error)
            }
            PaymentStorageError::Update(key) => {
                write!(f, "no value with key {} to update", key)
            }
            PaymentStorageError::Serialization(bincode_error) => {
                write!(f, "(de)serialization error: {}", bincode_error)
            }
        }
    }
}

impl Error for PaymentStorageError {}
