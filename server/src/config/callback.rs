use serde::{Deserialize, Serialize};

#[derive(Deserialize, PartialEq, Eq, Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct CallbackConfig {
    /// Number of callbacks that can be queued.
    pub queue_size: usize,
    /// Maximum number of times a callback can be retried. Infinite if left
    /// blank.
    pub max_retries: Option<usize>,
}

impl Default for CallbackConfig {
    fn default() -> Self {
        Self {
            queue_size: 1_000,
            max_retries: Some(50),
        }
    }
}
