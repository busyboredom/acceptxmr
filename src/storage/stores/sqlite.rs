use std::fmt::Display;

use log::{debug, trace, warn};
use sqlite::{version, Connection, ConnectionWithFullMutex, CursorWithOwnership, State, Value};
use thiserror::Error;

use crate::{storage::InvoiceStorage, Invoice, InvoiceId, SubIndex};

/// `SQLite` database containing pending invoices.
pub struct Sqlite {
    db: ConnectionWithFullMutex,
    table: TableName,
}

impl Sqlite {
    /// Open a [`SQLite`](sqlite) database at the specified location, and use
    /// the specified table. Creates a new database if one does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the database could not be opened at the specified
    /// path.
    pub fn new(path: &str, table: &str) -> Result<Sqlite, SqliteStorageError> {
        let db = Connection::open_with_full_mutex(path)?;
        debug!("Connection to SQLite v{} database established", version());

        let escaped_table = TableName::new(table);

        db.execute(format!(
            "CREATE TABLE IF NOT EXISTS {escaped_table} (
                major_subindex  INTEGER NOT NULL,
                minor_subindex  INTEGER NOT NULL,
                creation_height BLOB NOT NULL,
                invoice  BLOB NOT NULL,
                PRIMARY KEY (major_subindex, minor_subindex, creation_height)
            );"
        ))?;

        Ok(Sqlite {
            db,
            table: escaped_table,
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
            self.table
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
            return Err(SqliteStorageError::DuplicateEntry);
        }
        Ok(())
    }

    fn remove(&mut self, invoice_id: InvoiceId) -> Result<Option<Invoice>, SqliteStorageError> {
        let mut statement = self.db.prepare(
            format!(
                "DELETE FROM {}
                WHERE major_subindex = :major AND minor_subindex = :minor AND creation_height = :height RETURNING invoice",
                self.table
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
            let Some(invoice) = self.get(invoice_id)? else { return Ok(None) };

            let mut update_stmt = self.db.prepare(
                format!(
                    "UPDATE {} SET invoice = :invoice 
                    WHERE major_subindex = :major AND minor_subindex = :minor AND creation_height = :height",
                    self.table
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
            self.table
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
        // Check get the existing value.
        let mut select_stmt = self.db.prepare(format!(
            "SELECT COUNT(*) FROM {}
            WHERE major_subindex = :major AND minor_subindex = :minor",
            self.table
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
            .prepare(format!("SELECT invoice FROM {}", self.table))?;
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

struct TableName(pub String);

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

/// An error occurring while storing or retrieving pending invoices from a
/// `sqlite` database.
#[derive(Error, Debug)]
pub enum SqliteStorageError {
    /// An error caused by the database, or some interaction with it.
    #[error("database error: {0}")]
    Database(#[from] sqlite::Error),
    /// Attempted to insert an invoice which already exists
    #[error("attempted to insert an invoice which already exists")]
    DuplicateEntry,
    /// Failed to serialize an [`Invoice`].
    #[error("Serialization error: {0}")]
    Serialize(#[from] bincode::error::EncodeError),
    /// Failed to deserialize an [`Invoice`].
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
