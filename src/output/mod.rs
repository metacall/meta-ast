pub mod dashboard;
pub mod emitter;
pub mod graph;
pub mod inspect;

use serde::Serialize;

/// Supported output serialization formats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    Json,
    Yaml,
}

impl OutputFormat {
    /// Serialize a value to the chosen format.
    pub fn serialize<T: Serialize>(&self, value: &T) -> anyhow::Result<String> {
        match self {
            Self::Json => Ok(serde_json::to_string_pretty(value)?),
            Self::Yaml => Ok(yaml_serde::to_string(value)?),
        }
    }
}
