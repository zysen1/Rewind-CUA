//! Uncertainty gate.
//!
//! Inspired by LEASH (arXiv:2610.22280): when the executor is unsure, hand the
//! step to a stronger model or to a person. This is not a reproduction of that
//! paper's models or experiments. The rule here is the small deterministic
//! check described in `REWIND.md`: low confidence or low sample agreement
//! escalates, and a dangerous action that is also very unsure goes to a person.

use crate::action::Proposal;
use crate::policy::Uncertainty;

/// Where an unsure proposal should go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Escalation {
    /// Run the proposal.
    Proceed,
    /// Ask a stronger model for a new proposal. Do not run this one.
    StrongerModel { reason: String },
    /// Ask a person. Do not run this one until they say yes.
    Human { reason: String },
}

/// Decide from confidence, sample agreement, and risk.
pub fn decide(cfg: &Uncertainty, proposal: &Proposal) -> Escalation {
    let agreement = proposal.agreement();
    let low_confidence = proposal.confidence < cfg.escalate_below;
    let low_agreement = agreement < cfg.agreement_below;
    if !low_confidence && !low_agreement {
        return Escalation::Proceed;
    }
    let dangerous = proposal.action.risk().is_dangerous();
    if dangerous && proposal.confidence < cfg.human_below {
        return Escalation::Human {
            reason: format!(
                "confidence {:.2} and agreement {:.2} on a {} action",
                proposal.confidence,
                agreement,
                proposal.action.risk().as_str()
            ),
        };
    }
    Escalation::StrongerModel {
        reason: format!(
            "confidence {:.2}, agreement {:.2}",
            proposal.confidence, agreement
        ),
    }
}

/// Used when the stronger model is still unsure about a dangerous action.
pub fn still_needs_human(cfg: &Uncertainty, proposal: &Proposal) -> bool {
    proposal.action.risk().is_dangerous() && proposal.confidence < cfg.human_below
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, Proposal};

    fn proposal(action: Action, confidence: f64, samples: &[&str]) -> Proposal {
        Proposal {
            action,
            confidence,
            samples: samples.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    fn cfg() -> Uncertainty {
        Uncertainty::default()
    }

    #[test]
    fn confident_agreement_proceeds() {
        let action = Action::DeleteFile {
            path: "notes.txt".into(),
        };
        let proposal = proposal(action, 0.82, &["delete_file:notes.txt"; 3]);
        assert_eq!(decide(&cfg(), &proposal), Escalation::Proceed);
    }

    #[test]
    fn threshold_is_not_below() {
        let action = Action::WriteFile {
            path: "a.txt".into(),
            contents: "a".into(),
        };
        let proposal = proposal(action, 0.70, &["write_file:a.txt"; 3]);
        assert_eq!(decide(&cfg(), &proposal), Escalation::Proceed);
    }

    #[test]
    fn low_confidence_reversible_goes_to_a_stronger_model() {
        let action = Action::WriteFile {
            path: "a.txt".into(),
            contents: "a".into(),
        };
        let proposal = proposal(
            action,
            0.50,
            &["write_file:a.txt", "noop", "write_file:b.txt"],
        );
        assert!(matches!(
            decide(&cfg(), &proposal),
            Escalation::StrongerModel { .. }
        ));
    }

    #[test]
    fn very_unsure_delete_goes_to_a_person() {
        let action = Action::DeleteFile {
            path: "report.txt".into(),
        };
        let proposal = proposal(action, 0.20, &["delete_file:report.txt", "noop", "noop"]);
        assert!(matches!(
            decide(&cfg(), &proposal),
            Escalation::Human { .. }
        ));
    }
}
