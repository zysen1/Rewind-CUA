//! Errors from the journal, policy loader, workspace, and snapshot commands.

use std::path::Path;

/// Failure from the Rewind-CUA harness.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A rule, path, chain, or command failed.
    #[error("{0}")]
    Message(String),
    /// JSON encode or decode failed.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// YAML decode failed.
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
    /// A policy regular expression failed to compile.
    #[error("regex: {0}")]
    Regex(#[from] regex::Error),
}

impl Error {
    pub(crate) fn msg(text: impl Into<String>) -> Self {
        Self::Message(text.into())
    }

    pub(crate) fn io(path: &Path, err: std::io::Error) -> Self {
        Self::Message(format!("{}: {err}", path.display()))
    }
}
