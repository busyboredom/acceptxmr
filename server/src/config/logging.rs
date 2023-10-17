use log::LevelFilter;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, PartialEq, Eq, Clone, Copy, Debug, Serialize)]
pub struct LoggingConfig {
    pub verbosity: LevelFilter,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            verbosity: LevelFilter::Info,
        }
    }
}
