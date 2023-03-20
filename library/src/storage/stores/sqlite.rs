use std::fmt::Display;

use log::{debug, trace, warn};
use sqlite::{version, Connection, ConnectionWithFullMutex, CursorWithOwnership, State, Value};
use thiserror::Error;

use crate::{
    storage::{HeightStorage, InvoiceStorage, OutputId, OutputKeyStorage, OutputPubKey, Storage},
    Invoice, InvoiceId, SubIndex,
};

/// `SQLite` database.
pub struct Sqlite {
    db: ConnectionWithFullMutex,
    invoices: TableName,
    output_keys: TableName,
    height: TableName,
}

impl Sqlite {
    /// Open a [`SQLite`](sqlite) database at the specified location, and use
    /// the specified tables. Creates a new database if one does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the database could not be opened at the specified
    /// path.
    pub fn new(
        path: &str,
        invoice_table: &str,
        output_key_table: &str,
        height_table: &str,
    ) -> Result<Sqlite, SqliteStorageError> {
        let db = Connection::open_with_full_mutex(path)?;
        debug!("Connection to SQLite v{} database established", version());

        let invoices = TableName::new(invoice_table);
        let output_keys = TableName::new(output_key_table);
        let height = TableName::new(height_table);

        db.execute(format!(
            "CREATE TABLE IF NOT EXISTS {invoices} (
                major_subindex  INTEGER NOT NULL,
                minor_subindex  INTEGER NOT NULL,
                creation_height BLOB NOT NULL,
                invoice         BLOB NOT NULL,
                PRIMARY KEY (major_subindex, minor_subindex, creation_height)
            );"
        ))?;

        db.execute(format!(
            "CREATE TABLE IF NOT EXISTS {output_keys} (
                output_key BLOB NOT NULL,
                output_id  BLOB NOT NULL,
                PRIMARY KEY (output_key)
            );"
        ))?;

        db.execute(format!(
            "CREATE TABLE IF NOT EXISTS {height} (
                id INTEGER NOT NULL PRIMARY KEY,
                height BLOB NOT NULL,
                CHECK (id = 0)
            );"
        ))?;

        Ok(Sqlite {
            db,
            invoices,
            output_keys,
            height,
        })
    }
}

impl InvoiceStorage for Sqlite {
    type Error = SqliteStorageError;
    type Iter<'a> = SqliteIter<'a>;

    fn insert(&mut self, invoice: Invoice) -> Result<(), SqliteStorageError> {
        let invoice_id = invoice.id();

        // Prepare value (invoice).
        let value = bincode::encode_to_vec(invoice, bincode::config::standard())?;

        let mut statement = self.db.prepare(format!(
            "INSERT INTO {} (major_subindex, minor_subindex, creation_height, invoice) 
            VALUES (:major, :minor, :height, :invoice);",
            self.invoices
        ))?;
        statement.bind::<&[(_, Value)]>(
            &[
                // Cast to i64 is needed because `Value` doesn't support u32.
                (":major", i64::from(invoice_id.sub_index.major).into()),
                (":minor", i64::from(invoice_id.sub_index.minor).into()),
                (
                    ":height",
                    invoice_id.creation_height.to_be_bytes()[..].into(),
                ),
                (":invoice", value.into()),
            ][..],
        )?;

        while let Ok(State::Row) = statement.next() {
            warn!(
                "Invoice insertion returned an unexpected row: {:?}",
                statement.read::<Value, _>(0)?
            );
        }

        if self.db.change_count() == 0 {
            return Err(SqliteStorageError::DuplicateInvoice);
        }
        Ok(())
    }

    fn remove(&mut self, invoice_id: InvoiceId) -> Result<Option<Invoice>, SqliteStorageError> {
        let mut statement = self.db.prepare(
            format!(
                "DELETE FROM {}
                WHERE major_subindex = :major AND minor_subindex = :minor AND creation_height = :height RETURNING invoice",
                self.invoices
            )
        )?;
        statement.bind::<&[(_, Value)]>(
            &[
                // Cast to i64 is needed because `Value` doesn't support u32.
                (":major", i64::from(invoice_id.sub_index.major).into()),
                (":minor", i64::from(invoice_id.sub_index.minor).into()),
                (
                    ":height",
                    invoice_id.creation_height.to_be_bytes()[..].into(),
                ),
            ][..],
        )?;

        if statement.next()? == State::Done {
            return Ok(None);
        }
        let invoice_bytes = statement.read::<Vec<u8>, _>("invoice")?;
        if statement.next()? != State::Done {
            warn!(
                "Deletion of invoice returned more than one row: {:?}",
                statement.read::<Value, _>("invoice")?
            );
        }

        Ok(Some(
            bincode::decode_from_slice(&invoice_bytes, bincode::config::standard())?.0,
        ))
    }

    fn update(&mut self, invoice: Invoice) -> Result<Option<Invoice>, SqliteStorageError> {
        let invoice_id = invoice.id();

        // Prepare value.
        let value = bincode::encode_to_vec(invoice, bincode::config::standard())?;

        self.db.execute("BEGIN")?;

        let transaction = {
            let Some(invoice) = InvoiceStorage::get(self, invoice_id)? else {
                return Ok(None);
            };

            let mut update_stmt = self.db.prepare(
                format!(
                    "UPDATE {} SET invoice = :invoice 
                    WHERE major_subindex = :major AND minor_subindex = :minor AND creation_height = :height",
                    self.invoices
                )
            )?;
            update_stmt.bind::<&[(_, Value)]>(
                &[
                    (":invoice", value.into()),
                    // Cast to i64 is needed because `Value` doesn't support u32.
                    (":major", i64::from(invoice_id.sub_index.major).into()),
                    (":minor", i64::from(invoice_id.sub_index.minor).into()),
                    (
                        ":height",
                        invoice_id.creation_height.to_be_bytes()[..].into(),
                    ),
                ][..],
            )?;
            while State::Row == update_stmt.next()? {
                trace!(
                    "Invoice updated. Rows affected: {}",
                    update_stmt.read::<i64, _>(0)?
                );
            }

            Ok(invoice)
        };

        match transaction {
            Ok(inv) => {
                self.db.execute("COMMIT")?;
                Ok(Some(inv))
            }
            Err(e) => {
                self.db.execute("ROLLBACK")?;
                Err(e)
            }
        }
    }

    fn get(&self, invoice_id: InvoiceId) -> Result<Option<Invoice>, SqliteStorageError> {
        // Check get the existing value.
        let mut select_stmt = self.db.prepare(format!(
            "SELECT invoice FROM {}
            WHERE major_subindex = :major AND minor_subindex = :minor AND creation_height = :height",
            self.invoices
        ))?;
        select_stmt.bind::<&[(_, Value)]>(
            &[
                // Cast to i64 is needed because `Value` doesn't support u32.
                (":major", i64::from(invoice_id.sub_index.major).into()),
                (":minor", i64::from(invoice_id.sub_index.minor).into()),
                // Cast to byte array is needed because `Value` doesn't support u64.
                (
                    ":height",
                    invoice_id.creation_height.to_be_bytes()[..].into(),
                ),
            ][..],
        )?;

        if select_stmt.next()? == State::Done {
            return Ok(None);
        }
        let invoice_bytes = select_stmt.read::<Vec<u8>, _>("invoice")?;
        if select_stmt.next()? != State::Done {
            warn!(
                "Invoice query returned more than one row: {:?}",
                select_stmt.read::<Value, _>("invoice")?
            );
        }

        Ok(Some(
            bincode::decode_from_slice(&invoice_bytes, bincode::config::standard())?.0,
        ))
    }

    fn contains_sub_index(&self, sub_index: SubIndex) -> Result<bool, SqliteStorageError> {
        // Get the existing value.
        let mut select_stmt = self.db.prepare(format!(
            "SELECT COUNT(*) FROM {}
            WHERE major_subindex = :major AND minor_subindex = :minor",
            self.invoices
        ))?;
        select_stmt.bind::<&[(_, Value)]>(
            &[
                // Cast to i64 is needed because `Value` doesn't support u32.
                (":major", i64::from(sub_index.major).into()),
                (":minor", i64::from(sub_index.minor).into()),
            ][..],
        )?;
        if select_stmt.next()? == State::Done {
            return Ok(false);
        }
        let count = select_stmt.read::<i64, _>(0)?;
        if select_stmt.next()? != State::Done {
            warn!(
                "Invoice query returned more than one row: {:?}",
                select_stmt.read::<Value, _>("invoice")?
            );
        }

        Ok(count > 0)
    }

    fn try_iter(&self) -> Result<Self::Iter<'_>, SqliteStorageError> {
        let statement = self
            .db
            .prepare(format!("SELECT invoice FROM {}", self.invoices))?;
        Ok(SqliteIter(statement.into_iter()))
    }
}

pub struct SqliteIter<'a>(CursorWithOwnership<'a>);

impl<'stmt> Iterator for SqliteIter<'stmt> {
    type Item = Result<Invoice, SqliteStorageError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.0.next()? {
            Ok(row) => {
                let value = match row.try_read("invoice") {
                    Ok(v) => v,
                    Err(e) => return Some(Err(SqliteStorageError::from(e))),
                };
                let invoice_or_err = bincode::decode_from_slice(value, bincode::config::standard())
                    .map(|v| v.0)
                    .map_err(SqliteStorageError::Deserialize);
                Some(invoice_or_err)
            }
            Err(e) => Some(Err(SqliteStorageError::Database(e))),
        }
    }
}

impl OutputKeyStorage for Sqlite {
    type Error = SqliteStorageError;

    fn insert(&mut self, key: OutputPubKey, output_id: OutputId) -> Result<(), Self::Error> {
        let value = bincode::encode_to_vec(output_id, bincode::config::standard())?;

        let mut statement = self.db.prepare(format!(
            "INSERT INTO {} (output_key, output_id) 
            VALUES (:output_key, :output_id);",
            self.output_keys
        ))?;
        statement.bind::<&[(_, Value)]>(
            &[(":output_key", key.into()), (":output_id", value.into())][..],
        )?;

        while let Ok(State::Row) = statement.next() {
            warn!(
                "Output key insertion returned an unexpected row: {:?}",
                statement.read::<Value, _>(0)?
            );
        }

        if self.db.change_count() == 0 {
            return Err(SqliteStorageError::DuplicateOutputKey);
        }
        Ok(())
    }

    fn get(&self, key: OutputPubKey) -> Result<Option<OutputId>, Self::Error> {
        // Get the existing value.
        let mut select_stmt = self.db.prepare(format!(
            "SELECT output_id FROM {}
            WHERE output_key = :output_key",
            self.output_keys
        ))?;
        select_stmt.bind::<&[(_, Value)]>(&[(":output_key", key.into())][..])?;

        if select_stmt.next()? == State::Done {
            return Ok(None);
        }
        let output_id_bytes = select_stmt.read::<Vec<u8>, _>("output_id")?;
        if select_stmt.next()? != State::Done {
            warn!(
                "Output key query returned more than one row: {:?}",
                select_stmt.read::<Value, _>(0)?
            );
        }

        Ok(Some(
            bincode::decode_from_slice(&output_id_bytes, bincode::config::standard())?.0,
        ))
    }
}

impl From<OutputPubKey> for Value {
    fn from(value: OutputPubKey) -> Self {
        Value::Binary(value.0.to_vec())
    }
}

impl HeightStorage for Sqlite {
    type Error = SqliteStorageError;

    fn upsert(&mut self, height: u64) -> Result<Option<u64>, Self::Error> {
        let encoded_height = bincode::encode_to_vec(height, bincode::config::standard())?;

        self.db.execute("BEGIN")?;

        let transaction = {
            let old_height = HeightStorage::get(self)?;

            let mut update_stmt = self.db.prepare(format!(
                "INSERT INTO {} (id, height) 
                VALUES (0, :height)
                ON CONFLICT (id) DO UPDATE SET height=:height;",
                self.height
            ))?;
            update_stmt.bind::<&[(_, Value)]>(&[(":height", encoded_height.into())][..])?;
            while State::Row == update_stmt.next()? {
                trace!(
                    "Height updated. Rows affected: {}",
                    update_stmt.read::<i64, _>(0)?
                );
            }

            Ok(old_height)
        };

        match transaction {
            Ok(inv) => {
                self.db.execute("COMMIT")?;
                Ok(inv)
            }
            Err(e) => {
                self.db.execute("ROLLBACK")?;
                Err(e)
            }
        }
    }

    fn get(&self) -> Result<Option<u64>, Self::Error> {
        // Get the existing value.
        let mut select_stmt = self.db.prepare(format!(
            "SELECT height FROM {}
            WHERE id = 0",
            self.height
        ))?;

        if select_stmt.next()? == State::Done {
            return Ok(None);
        }
        let height_bytes = select_stmt.read::<Vec<u8>, _>("height")?;
        if select_stmt.next()? != State::Done {
            warn!(
                "Height query returned more than one row: {:?}",
                select_stmt.read::<Value, _>(0)?
            );
        }

        Ok(Some(
            bincode::decode_from_slice(&height_bytes, bincode::config::standard())?.0,
        ))
    }
}

impl Storage for Sqlite {
    type Error = SqliteStorageError;
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Ord, PartialOrd)]
struct TableName(String);

impl TableName {
    fn new(table: &str) -> TableName {
        TableName(format!("\"{}\"", table.replace('\"', "\"\"")))
    }
}

impl Display for TableName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// An error occurring while storing or retrieving values from a
/// `sqlite` database.
#[derive(Error, Debug)]
pub enum SqliteStorageError {
    /// An error caused by the database, or some interaction with it.
    #[error("database error: {0}")]
    Database(#[from] sqlite::Error),
    /// Attempted to insert an invoice which already exists
    #[error("attempted to insert an invoice which already exists")]
    DuplicateInvoice,
    /// Attempted to insert an output key which already exists
    #[error("attempted to insert an output public key which already exists")]
    DuplicateOutputKey,
    /// Failed to serialize an [`Invoice`] or [`OutputPubKey`].
    #[error("Serialization error: {0}")]
    Serialize(#[from] bincode::error::EncodeError),
    /// Failed to deserialize an [`Invoice`] or [`OutputPubKey`].
    #[error("Deserialization error: {0}")]
    Deserialize(#[from] bincode::error::DecodeError),
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    use super::TableName;

    #[test_case("" => "\"\"")]
    #[test_case("invoices" => "\"invoices\"")]
    #[test_case("\"doublequote\"" => "\"\"\"doublequote\"\"\"")]
    #[test_case("\"onequote" => "\"\"\"onequote\"")]
    #[test_case("under_score" => "\"under_score\"")]
    fn escape_table_name(table: &str) -> String {
        TableName::new(table).0
    }
}
