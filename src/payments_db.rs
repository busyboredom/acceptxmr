use crate::Payment;

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

    pub fn flush(&self) {
        self.0.flush().expect("Failed to flush payment updates to payments database");
    }
}
