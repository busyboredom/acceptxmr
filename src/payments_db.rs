use std::fmt;

use crate::{Payment, SubIndex};

/// Database containing pending payments.
pub struct PaymentsDb(sled::Db);

impl PaymentsDb {
    pub fn new() -> PaymentsDb {
        let db = sled::Config::default()
            .path("PaymentsDb")
            .flush_every_ms(None)
            .open()
            .unwrap();
        PaymentsDb(db)
    }

    pub fn insert(
        &mut self,
        payment: &Payment,
    ) -> Result<Option<Payment>, Box<dyn std::error::Error>> {
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

    pub fn get(
        &self,
        subaddress_index: &SubIndex,
    ) -> Result<Option<Payment>, PaymentStorageErrorKind> {
        // Prepare key (subaddress index).
        let key = [
            subaddress_index.major.to_be_bytes(),
            subaddress_index.minor.to_be_bytes(),
        ]
        .concat();

        let maybe_payment_ivec = self.0.get(&key)?;
        match maybe_payment_ivec {
            Some(payment_ivec) => Ok(Some(bincode::deserialize(&payment_ivec)?)),
            None => Ok(None),
        }
    }

    pub fn iter(
        &self,
    ) -> impl DoubleEndedIterator<Item = Result<Payment, PaymentStorageErrorKind>> + Send + Sync
    {
        // Convert iterator of Result<IVec> to Result<Payment>.
        self.0
            .iter()
            .values()
            .map(|r| {
                r.map(|ivec| bincode::deserialize(&ivec).map_err(PaymentStorageErrorKind::from))
                    .map_err(PaymentStorageErrorKind::from)
            })
            .flatten()
    }

    pub fn contains_key(
        &self,
        subaddress_index: &SubIndex,
    ) -> Result<bool, PaymentStorageErrorKind> {
        // Prepare key (subaddress index).
        let key = [
            subaddress_index.major.to_be_bytes(),
            subaddress_index.minor.to_be_bytes(),
        ]
        .concat();

        self.0
            .contains_key(key)
            .map_err(PaymentStorageErrorKind::from)
    }

    pub fn new_batch() -> Batch {
        Batch::new()
    }

    pub fn apply_batch(&self, batch: Batch) -> Result<(), PaymentStorageErrorKind> {
        Ok(self.0.apply_batch(batch.0)?)
    }

    pub fn flush(&self) {
        self.0
            .flush()
            .expect("Failed to flush payment updates to payments database");
    }
}

pub struct Batch(sled::Batch);

impl Batch {
    fn new() -> Batch {
        Batch(sled::Batch::default())
    }

    pub fn insert(&mut self, payment: &Payment) -> Result<(), PaymentStorageErrorKind> {
        // Prepare key (subaddress index).
        let key = [
            payment.index.major.to_be_bytes(),
            payment.index.minor.to_be_bytes(),
        ]
        .concat();

        // Prepare value (payment).
        let value = bincode::serialize(&payment)?;
        // Insert the payment into the database.
        self.0.insert(key, value);

        Ok(())
    }
}

#[derive(Debug)]
pub enum PaymentStorageErrorKind {
    DatabaseError(sled::Error),
    SerializationError(bincode::Error),
}

impl From<sled::Error> for PaymentStorageErrorKind {
    fn from(e: sled::Error) -> Self {
        Self::DatabaseError(e)
    }
}

impl From<bincode::Error> for PaymentStorageErrorKind {
    fn from(e: bincode::Error) -> Self {
        Self::SerializationError(e)
    }
}

impl fmt::Display for PaymentStorageErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaymentStorageErrorKind::DatabaseError(sled_error) => {
                write!(f, "Database error: {}", sled_error)
            }
            PaymentStorageErrorKind::SerializationError(bincode_error) => {
                write!(f, "(De)serialization error: {}", bincode_error)
            }
        }
    }
}
