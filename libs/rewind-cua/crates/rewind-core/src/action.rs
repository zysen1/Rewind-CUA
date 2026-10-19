//! Actions the harness can execute inside a workspace.

use serde::{Deserialize, Serialize};

/// How hard an effect is to undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// No effect.
    Safe,
    /// A file write or move. A workspace snapshot can undo it.
    Reversible,
    /// A delete. A workspace snapshot can undo it; a guest without a snapshot cannot.
    Irreversible,
    /// Mail or payment. No snapshot puts this back.
    External,
}

impl Risk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Reversible => "reversible",
            Self::Irreversible => "irreversible",
            Self::External => "external",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "safe" => Some(Self::Safe),
            "reversible" => Some(Self::Reversible),
            "irreversible" => Some(Self::Irreversible),
            "external" => Some(Self::External),
            _ => None,
        }
    }

    pub fn is_dangerous(self) -> bool {
        matches!(self, Self::Irreversible | Self::External)
    }
}

/// One computer-use action against the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Create or replace a file.
    WriteFile { path: String, contents: String },
    /// Delete a file.
    DeleteFile { path: String },
    /// Move a file inside the workspace.
    MoveFile { from: String, to: String },
    /// Send mail. The effect is recorded in the outbox and is not rewound.
    SendEmail { to: String, subject: String },
    /// Pay someone. The effect is recorded in the outbox and is not rewound.
    Pay { payee: String, amount_cents: u64 },
    /// Leave the workspace unchanged.
    Noop,
}

impl Action {
    /// Stable id used for sample agreement.
    pub fn fingerprint(&self) -> String {
        match self {
            Self::WriteFile { path, .. } => format!("write_file:{path}"),
            Self::DeleteFile { path } => format!("delete_file:{path}"),
            Self::MoveFile { from, to } => format!("move_file:{from}->{to}"),
            Self::SendEmail { to, .. } => format!("send_email:{to}"),
            Self::Pay {
                payee,
                amount_cents,
            } => format!("pay:{payee}:{amount_cents}"),
            Self::Noop => "noop".to_string(),
        }
    }

    pub fn risk(&self) -> Risk {
        match self {
            Self::WriteFile { .. } | Self::MoveFile { .. } => Risk::Reversible,
            Self::DeleteFile { .. } => Risk::Irreversible,
            Self::SendEmail { .. } | Self::Pay { .. } => Risk::External,
            Self::Noop => Risk::Safe,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::WriteFile { .. } => "write_file",
            Self::DeleteFile { .. } => "delete_file",
            Self::MoveFile { .. } => "move_file",
            Self::SendEmail { .. } => "send_email",
            Self::Pay { .. } => "pay",
            Self::Noop => "noop",
        }
    }

    /// Strings a path rule can match.
    pub fn match_targets(&self) -> Vec<&str> {
        match self {
            Self::WriteFile { path, .. } | Self::DeleteFile { path } => vec![path.as_str()],
            Self::MoveFile { from, to } => vec![from.as_str(), to.as_str()],
            Self::SendEmail { to, .. } => vec![to.as_str()],
            Self::Pay { payee, .. } => vec![payee.as_str()],
            Self::Noop => Vec::new(),
        }
    }
}

/// A model proposal: the action, its confidence, and the other samples.
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    pub action: Action,
    /// Executor confidence in `[0, 1]`.
    pub confidence: f64,
    /// Fingerprints from repeated samples. Agreement is the share equal to `action`.
    pub samples: Vec<String>,
}

impl Proposal {
    /// Fraction of samples whose fingerprint equals the chosen action.
    ///
    /// An empty sample list is treated as full agreement so a caller that did
    /// not sample cannot accidentally look uncertain. The task loader rejects
    /// that case for eval tasks.
    pub fn agreement(&self) -> f64 {
        if self.samples.is_empty() {
            return 1.0;
        }
        let fp = self.action.fingerprint();
        let hits = self.samples.iter().filter(|sample| *sample == &fp).count();
        hits as f64 / self.samples.len() as f64
    }
}

/// YAML shape of an action, converted after load.
#[derive(Debug, Deserialize)]
pub(crate) struct ActionFile {
    pub kind: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub contents: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub dest: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub amount_cents: Option<u64>,
}

impl ActionFile {
    pub(crate) fn into_action(self) -> Result<Action, super::Error> {
        let missing = |field: &str| super::Error::msg(format!("{} requires {field}", self.kind));
        match self.kind.as_str() {
            "write_file" => Ok(Action::WriteFile {
                path: self.path.ok_or_else(|| missing("path"))?,
                contents: self.contents.unwrap_or_default(),
            }),
            "delete_file" => Ok(Action::DeleteFile {
                path: self.path.ok_or_else(|| missing("path"))?,
            }),
            "move_file" => Ok(Action::MoveFile {
                from: self.from.ok_or_else(|| missing("from"))?,
                to: self.to.ok_or_else(|| missing("to"))?,
            }),
            "send_email" => Ok(Action::SendEmail {
                to: self.dest.ok_or_else(|| missing("dest"))?,
                subject: self.subject.unwrap_or_default(),
            }),
            "pay" => Ok(Action::Pay {
                payee: self.dest.ok_or_else(|| missing("dest"))?,
                amount_cents: self.amount_cents.ok_or_else(|| missing("amount_cents"))?,
            }),
            "noop" => Ok(Action::Noop),
            other => Err(super::Error::msg(format!("unknown action kind {other}"))),
        }
    }
}
