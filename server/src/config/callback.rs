use serde::{Deserialize, Serialize};

#[derive(Deserialize, PartialEq, Eq, Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct CallbackConfig {
    /// Number of callbacks that can be queued.
    pub queue_size: usize,
}

impl Default for CallbackConfig {
    fn default() -> Self {
        Self { queue_size: 10_000 }
    }
}
