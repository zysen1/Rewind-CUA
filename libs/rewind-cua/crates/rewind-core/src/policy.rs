//! YAML policy: forbidden actions and actions that need a person.

use std::fs;
use std::path::Path;

use regex::Regex;
use serde::Deserialize;

use crate::action::{Action, Risk};
use crate::Error;

/// What the policy says about one action, before it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyDecision {
    /// The action may run, subject to the uncertainty gate.
    Allow,
    /// The action is forbidden. The harness does not execute it.
    Deny { rule: String, reason: String },
    /// A person must approve the action before it runs.
    NeedsApproval { rule: String, reason: String },
}

/// Uncertainty thresholds. Comparisons are strict: a value equal to the
/// threshold does not escalate.
#[derive(Clone, Debug)]
pub struct Uncertainty {
    pub escalate_below: f64,
    pub agreement_below: f64,
    pub human_below: f64,
}

impl Default for Uncertainty {
    fn default() -> Self {
        Self {
            escalate_below: 0.70,
            agreement_below: 0.67,
            human_below: 0.45,
        }
    }
}

/// Loaded policy.
#[derive(Clone, Debug)]
pub struct Policy {
    pub uncertainty: Uncertainty,
    pub checkpoint_risks: Vec<Risk>,
    pub max_attempts: u32,
    forbidden: Vec<Rule>,
    approval: Vec<Rule>,
}

#[derive(Clone, Debug)]
struct Rule {
    id: String,
    kind: Option<String>,
    path: Option<Regex>,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct PolicyFile {
    version: u32,
    #[serde(default)]
    forbidden: Vec<RuleFile>,
    #[serde(default)]
    require_approval: Vec<RuleFile>,
    #[serde(default)]
    uncertainty: UncertaintyFile,
    #[serde(default)]
    checkpoint: CheckpointFile,
    #[serde(default)]
    retry: RetryFile,
}

#[derive(Debug, Deserialize)]
struct RuleFile {
    id: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    path_regex: Option<String>,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct UncertaintyFile {
    #[serde(default = "default_escalate")]
    escalate_below: f64,
    #[serde(default = "default_agreement")]
    agreement_below: f64,
    #[serde(default = "default_human")]
    human_below: f64,
}

impl Default for UncertaintyFile {
    fn default() -> Self {
        Self {
            escalate_below: default_escalate(),
            agreement_below: default_agreement(),
            human_below: default_human(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct CheckpointFile {
    #[serde(default = "default_before")]
    before: Vec<String>,
}

impl Default for CheckpointFile {
    fn default() -> Self {
        Self {
            before: default_before(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RetryFile {
    #[serde(default = "default_attempts")]
    max_attempts: u32,
}

impl Default for RetryFile {
    fn default() -> Self {
        Self {
            max_attempts: default_attempts(),
        }
    }
}

fn default_escalate() -> f64 {
    0.70
}
fn default_agreement() -> f64 {
    0.67
}
fn default_human() -> f64 {
    0.45
}
fn default_attempts() -> u32 {
    2
}
fn default_before() -> Vec<String> {
    vec![
        "reversible".to_string(),
        "irreversible".to_string(),
        "external".to_string(),
    ]
}

impl Policy {
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = fs::read_to_string(path).map_err(|err| Error::io(path, err))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, Error> {
        let file: PolicyFile = serde_yaml::from_str(text)?;
        if file.version != 1 {
            return Err(Error::msg(format!(
                "policy version {} is not supported (want 1)",
                file.version
            )));
        }
        let mut checkpoint_risks = Vec::new();
        for word in &file.checkpoint.before {
            let risk = Risk::parse(word)
                .ok_or_else(|| Error::msg(format!("unknown checkpoint risk {word}")))?;
            checkpoint_risks.push(risk);
        }
        if file.retry.max_attempts == 0 {
            return Err(Error::msg("retry.max_attempts must be at least 1"));
        }
        let uncertainty = Uncertainty {
            escalate_below: unit(file.uncertainty.escalate_below, "escalate_below")?,
            agreement_below: unit(file.uncertainty.agreement_below, "agreement_below")?,
            human_below: unit(file.uncertainty.human_below, "human_below")?,
        };
        Ok(Self {
            uncertainty,
            checkpoint_risks,
            max_attempts: file.retry.max_attempts,
            forbidden: compile_rules(file.forbidden)?,
            approval: compile_rules(file.require_approval)?,
        })
    }

    pub fn decide(&self, action: &Action) -> PolicyDecision {
        if let Some(rule) = self.find(&self.forbidden, action) {
            return PolicyDecision::Deny {
                rule: rule.id.clone(),
                reason: rule.reason.clone(),
            };
        }
        if let Some(rule) = self.find(&self.approval, action) {
            return PolicyDecision::NeedsApproval {
                rule: rule.id.clone(),
                reason: rule.reason.clone(),
            };
        }
        PolicyDecision::Allow
    }

    pub fn checkpoint_before(&self, risk: Risk) -> bool {
        self.checkpoint_risks.contains(&risk)
    }

    fn find<'a>(&self, rules: &'a [Rule], action: &Action) -> Option<&'a Rule> {
        rules.iter().find(|rule| rule.matches(action))
    }
}

impl Rule {
    fn matches(&self, action: &Action) -> bool {
        if let Some(kind) = &self.kind {
            if action.kind_name() != kind {
                return false;
            }
        }
        if let Some(path) = &self.path {
            let targets = action.match_targets();
            if !targets.iter().any(|target| path.is_match(target)) {
                return false;
            }
        }
        true
    }
}

fn compile_rules(files: Vec<RuleFile>) -> Result<Vec<Rule>, Error> {
    let mut rules = Vec::with_capacity(files.len());
    for file in files {
        if file.kind.is_none() && file.path_regex.is_none() {
            return Err(Error::msg(format!(
                "rule {} matches every action; set kind or path_regex",
                file.id
            )));
        }
        if file.reason.trim().is_empty() {
            return Err(Error::msg(format!("rule {} needs a reason", file.id)));
        }
        let path = match file.path_regex {
            Some(pattern) => Some(Regex::new(&pattern)?),
            None => None,
        };
        rules.push(Rule {
            id: file.id,
            kind: file.kind,
            path,
            reason: file.reason,
        });
    }
    Ok(rules)
}

fn unit(value: f64, name: &str) -> Result<f64, Error> {
    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(Error::msg(format!("{name} must be between 0 and 1")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;

    fn policy() -> Policy {
        Policy::parse(include_str!("../../../policy/default.yaml")).unwrap()
    }

    #[test]
    fn pay_is_forbidden_and_mail_needs_a_person() {
        let policy = policy();
        let pay = Action::Pay {
            payee: "vendor".into(),
            amount_cents: 100,
        };
        assert!(matches!(
            policy.decide(&pay),
            PolicyDecision::Deny { rule, .. } if rule == "no-payments"
        ));
        let mail = Action::SendEmail {
            to: "ada@example.com".into(),
            subject: "hi".into(),
        };
        assert!(matches!(
            policy.decide(&mail),
            PolicyDecision::NeedsApproval { rule, .. } if rule == "approve-email"
        ));
        let write = Action::WriteFile {
            path: "notes.txt".into(),
            contents: "x".into(),
        };
        assert_eq!(policy.decide(&write), PolicyDecision::Allow);
    }

    #[test]
    fn path_escape_is_forbidden() {
        let policy = policy();
        let action = Action::DeleteFile {
            path: "../etc/passwd".into(),
        };
        assert!(matches!(
            policy.decide(&action),
            PolicyDecision::Deny { .. }
        ));
    }
}
