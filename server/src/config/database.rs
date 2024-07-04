use std::{path::PathBuf, str::FromStr};

use serde::{Deserialize, Serialize};

/// Default invoice storage database directory.
const DEFAULT_DB_DIR: &str = "AcceptXMR_DB/";

#[derive(Clone, Deserialize, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct DatabaseConfig {
    pub path: PathBuf,
    /// Automatically delete expired invoices that aren't pending confirmation.
    pub delete_expired: bool,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from_str(DEFAULT_DB_DIR).unwrap(),
            delete_expired: true,
        }
    }
}
