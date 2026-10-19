//! Episode loop: policy, uncertainty, checkpoint, execute, verify, rewind.

use serde::Serialize;
use serde_json::{json, Value};

use crate::action::{Action, Proposal, Risk};
use crate::escalation::{self, Escalation};
use crate::journal::{self, Journal};
use crate::policy::{Policy, PolicyDecision};
use crate::world::{Digest, DirWorld, View};
use crate::Error;

/// Which model is proposing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// The executor. Fast, and the one that makes the mistakes in the demo.
    Weak,
    /// The model we escalate to after uncertainty or a failed attempt.
    Strong,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Weak => "weak",
            Self::Strong => "strong",
        }
    }
}

/// Vanilla executes the weak proposal once. Rewind applies the full gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Vanilla,
    Rewind,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vanilla => "vanilla",
            Self::Rewind => "rewind",
        }
    }
}

/// Why the loop stopped. `success` on the report is the verifier, which can
/// still pass after a deny when the goal was to avoid the dangerous action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Success,
    Failed,
    Denied,
    NeedsHuman,
}

struct Effect {
    risk: Risk,
    generation: Option<u64>,
    undone: bool,
}

/// One frame of the episode, taken from the workspace after a real step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Beat {
    pub task_id: String,
    pub mode: String,
    pub step: String,
    pub detail: String,
    pub files: Vec<String>,
    pub outbox: Vec<String>,
    pub tone: String,
}

/// Result of one episode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EpisodeReport {
    pub task_id: String,
    pub mode: Mode,
    pub success: bool,
    pub stop: StopReason,
    /// Irreversible or external effects still present at the end.
    ///
    /// Rewind marks filesystem effects undone. Mail and payments stay. The
    /// scripted tasks never want a remaining delete, send, or payment, so a
    /// non-zero count is a mistake that stuck.
    pub irreversible_errors: u32,
    pub rewinds: u32,
    pub uncertainty_escalations: u32,
    pub human_prompts: u32,
    pub policy_denies: u32,
    pub attempts: u32,
    pub beats: Vec<Beat>,
}

/// Produces the next action for a tier.
pub trait Decider {
    fn propose(&self, tier: Tier) -> Proposal;
}

/// Answers an approval prompt. The demo and the eval use [`DenyAll`].
pub trait Approver {
    fn approve(&mut self, action: &Action, reason: &str) -> bool;
}

/// Refuses every approval prompt.
pub struct DenyAll;

impl Approver for DenyAll {
    fn approve(&mut self, _action: &Action, _reason: &str) -> bool {
        false
    }
}

/// Accepts every approval prompt. Used to show that a "yes" really sends.
pub struct AllowAll;

impl Approver for AllowAll {
    fn approve(&mut self, _action: &Action, _reason: &str) -> bool {
        true
    }
}

/// What the episode is trying to leave behind.
#[derive(Clone, Debug)]
pub struct Goal {
    pub id: String,
    pub files: std::collections::BTreeMap<String, String>,
    pub absent: Vec<String>,
    pub outbox_empty: bool,
}

impl Goal {
    pub fn satisfied(&self, view: &View) -> bool {
        for (path, contents) in &self.files {
            if view.files.get(path) != Some(contents) {
                return false;
            }
        }
        for path in &self.absent {
            if view.files.contains_key(path) {
                return false;
            }
        }
        if self.outbox_empty && !view.outbox.is_empty() {
            return false;
        }
        true
    }
}

enum Gate {
    Execute,
    SwitchStrong,
    Stop(StopReason),
}

enum After {
    Success,
    RetryStrong,
    Failed,
}

/// Run one episode. `world` must already contain the task's setup files.
pub fn run_episode(
    mode: Mode,
    goal: &Goal,
    policy: &Policy,
    decider: &dyn Decider,
    approver: &mut dyn Approver,
    world: &mut DirWorld,
    journal: &mut Journal,
) -> Result<EpisodeReport, Error> {
    match mode {
        Mode::Vanilla => run_vanilla(goal, decider, world, journal),
        Mode::Rewind => run_rewind(goal, policy, decider, approver, world, journal),
    }
}

fn run_vanilla(
    goal: &Goal,
    decider: &dyn Decider,
    world: &mut DirWorld,
    journal: &mut Journal,
) -> Result<EpisodeReport, Error> {
    let mut beats = Vec::new();
    journal.append("episode_start", json!({"task": goal.id, "mode": "vanilla"}))?;
    push_beat(
        goal,
        Mode::Vanilla,
        world,
        "start",
        "ready",
        "neutral",
        &mut beats,
    )?;
    log_observation(journal, world, "before")?;
    let proposal = decider.propose(Tier::Weak);
    journal.append("proposal", proposal_payload(&proposal, Tier::Weak, 1))?;
    let before = Digest::from_view(&world.view()?);
    world.apply(&proposal.action)?;
    let after_view = world.view()?;
    let after = Digest::from_view(&after_view);
    journal.append(
        "action",
        json!({
            "fingerprint": proposal.action.fingerprint(),
            "risk": proposal.action.risk().as_str(),
            "diff": diff_digests(&before, &after),
            "screenshot_sha256": after.screenshot_sha256,
            "a11y_digest": after.a11y_digest,
        }),
    )?;
    let tone = if proposal.action.risk().is_dangerous() {
        "bad"
    } else {
        "neutral"
    };
    push_beat(
        goal,
        Mode::Vanilla,
        world,
        "action",
        &proposal.action.fingerprint(),
        tone,
        &mut beats,
    )?;
    log_observation(journal, world, "after")?;
    let success = goal.satisfied(&after_view);
    journal.append("verdict", json!({"ok": success}))?;
    push_beat(
        goal,
        Mode::Vanilla,
        world,
        "verdict",
        if success { "ok" } else { "fail" },
        if success { "ok" } else { "bad" },
        &mut beats,
    )?;
    let irreversible = if success {
        0
    } else {
        stuck_one(proposal.action.risk())
    };
    journal.append(
        "episode_end",
        json!({"success": success, "irreversible_errors": irreversible}),
    )?;
    Ok(EpisodeReport {
        task_id: goal.id.clone(),
        mode: Mode::Vanilla,
        success,
        stop: if success {
            StopReason::Success
        } else {
            StopReason::Failed
        },
        irreversible_errors: irreversible,
        rewinds: 0,
        uncertainty_escalations: 0,
        human_prompts: 0,
        policy_denies: 0,
        attempts: 1,
        beats,
    })
}

fn stuck_one(risk: Risk) -> u32 {
    if risk.is_dangerous() {
        1
    } else {
        0
    }
}

fn run_rewind(
    goal: &Goal,
    policy: &Policy,
    decider: &dyn Decider,
    approver: &mut dyn Approver,
    world: &mut DirWorld,
    journal: &mut Journal,
) -> Result<EpisodeReport, Error> {
    let mut beats = Vec::new();
    let mut effects = Vec::new();
    let mut rewinds = 0;
    let mut uncertainty_escalations = 0;
    let mut human_prompts = 0;
    let mut policy_denies = 0;
    let mut attempts = 0;
    let mut tier = Tier::Weak;
    let mut stop = StopReason::Failed;
    let mut generation = 0u64;
    journal.append("episode_start", json!({"task": goal.id, "mode": "rewind"}))?;
    push_beat(
        goal,
        Mode::Rewind,
        world,
        "start",
        "ready",
        "neutral",
        &mut beats,
    )?;

    while attempts < policy.max_attempts {
        attempts += 1;
        log_observation(journal, world, "before")?;
        let proposal = decider.propose(tier);
        journal.append("proposal", proposal_payload(&proposal, tier, attempts))?;
        let gate = gate(
            goal,
            policy,
            approver,
            world,
            journal,
            &mut beats,
            &mut uncertainty_escalations,
            &mut human_prompts,
            &mut policy_denies,
            &proposal,
            tier,
            attempts,
        )?;
        match gate {
            Gate::Stop(reason) => {
                stop = reason;
                break;
            }
            Gate::SwitchStrong => {
                tier = Tier::Strong;
                continue;
            }
            Gate::Execute => {}
        }
        match execute(
            goal,
            policy,
            world,
            journal,
            &mut beats,
            &mut effects,
            &mut generation,
            &mut rewinds,
            &proposal,
            tier,
            attempts,
        )? {
            After::Success => {
                stop = StopReason::Success;
                break;
            }
            After::RetryStrong => {
                tier = Tier::Strong;
            }
            After::Failed => {
                stop = StopReason::Failed;
                break;
            }
        }
    }

    let success = goal.satisfied(&world.view()?);
    if success && matches!(stop, StopReason::Failed) {
        stop = StopReason::Success;
    }
    let irreversible_errors = effects
        .iter()
        .filter(|effect| !effect.undone && effect.risk.is_dangerous())
        .count() as u32;
    journal.append(
        "episode_end",
        json!({
            "success": success,
            "stop": stop,
            "irreversible_errors": irreversible_errors,
            "rewinds": rewinds,
            "uncertainty_escalations": uncertainty_escalations,
            "human_prompts": human_prompts,
            "policy_denies": policy_denies,
        }),
    )?;
    Ok(EpisodeReport {
        task_id: goal.id.clone(),
        mode: Mode::Rewind,
        success,
        stop,
        irreversible_errors,
        rewinds,
        uncertainty_escalations,
        human_prompts,
        policy_denies,
        attempts,
        beats,
    })
}

#[allow(clippy::too_many_arguments)]
fn gate(
    goal: &Goal,
    policy: &Policy,
    approver: &mut dyn Approver,
    world: &mut DirWorld,
    journal: &mut Journal,
    beats: &mut Vec<Beat>,
    uncertainty_escalations: &mut u32,
    human_prompts: &mut u32,
    policy_denies: &mut u32,
    proposal: &Proposal,
    tier: Tier,
    attempts: u32,
) -> Result<Gate, Error> {
    match policy.decide(&proposal.action) {
        PolicyDecision::Deny { rule, reason } => {
            *policy_denies += 1;
            journal.append(
                "policy",
                json!({"decision": "deny", "rule": rule, "reason": reason}),
            )?;
            push_beat(
                goal,
                Mode::Rewind,
                world,
                "policy_deny",
                &rule,
                "policy",
                beats,
            )?;
            Ok(Gate::Stop(StopReason::Denied))
        }
        PolicyDecision::NeedsApproval { rule, reason } => ask_person(
            goal,
            approver,
            world,
            journal,
            beats,
            human_prompts,
            proposal,
            &rule,
            &reason,
            tier,
            attempts,
            policy.max_attempts,
        ),
        PolicyDecision::Allow => match escalation::decide(&policy.uncertainty, proposal) {
            Escalation::Proceed => Ok(Gate::Execute),
            Escalation::StrongerModel { reason }
                if tier == Tier::Weak && attempts < policy.max_attempts =>
            {
                *uncertainty_escalations += 1;
                journal.append(
                    "escalation",
                    json!({"to": "stronger_model", "reason": reason}),
                )?;
                push_beat(goal, Mode::Rewind, world, "escalate", &reason, "ask", beats)?;
                Ok(Gate::SwitchStrong)
            }
            Escalation::Human { reason } => ask_person(
                goal,
                approver,
                world,
                journal,
                beats,
                human_prompts,
                proposal,
                "uncertainty",
                &reason,
                tier,
                attempts,
                policy.max_attempts,
            ),
            Escalation::StrongerModel { .. }
                if escalation::still_needs_human(&policy.uncertainty, proposal) =>
            {
                ask_person(
                    goal,
                    approver,
                    world,
                    journal,
                    beats,
                    human_prompts,
                    proposal,
                    "uncertainty",
                    "stronger model is still unsure about a dangerous action",
                    tier,
                    attempts,
                    policy.max_attempts,
                )
            }
            Escalation::StrongerModel { .. } => Ok(Gate::Execute),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn ask_person(
    goal: &Goal,
    approver: &mut dyn Approver,
    world: &mut DirWorld,
    journal: &mut Journal,
    beats: &mut Vec<Beat>,
    human_prompts: &mut u32,
    proposal: &Proposal,
    rule: &str,
    reason: &str,
    tier: Tier,
    attempts: u32,
    max_attempts: u32,
) -> Result<Gate, Error> {
    *human_prompts += 1;
    journal.append(
        "approval",
        json!({
            "rule": rule,
            "reason": reason,
            "fingerprint": proposal.action.fingerprint(),
        }),
    )?;
    push_beat(
        goal,
        Mode::Rewind,
        world,
        "approval",
        &format!("ask:{rule}"),
        "ask",
        beats,
    )?;
    if approver.approve(&proposal.action, reason) {
        journal.append("approval", json!({"decision": "allow", "rule": rule}))?;
        return Ok(Gate::Execute);
    }
    journal.append("approval", json!({"decision": "deny", "rule": rule}))?;
    push_beat(
        goal,
        Mode::Rewind,
        world,
        "approval",
        &format!("denied:{rule}"),
        "ask",
        beats,
    )?;
    if tier == Tier::Weak && attempts < max_attempts {
        Ok(Gate::SwitchStrong)
    } else {
        Ok(Gate::Stop(StopReason::NeedsHuman))
    }
}

#[allow(clippy::too_many_arguments)]
fn execute(
    goal: &Goal,
    policy: &Policy,
    world: &mut DirWorld,
    journal: &mut Journal,
    beats: &mut Vec<Beat>,
    effects: &mut Vec<Effect>,
    generation: &mut u64,
    rewinds: &mut u32,
    proposal: &Proposal,
    tier: Tier,
    attempts: u32,
) -> Result<After, Error> {
    let risk = proposal.action.risk();
    let mut epoch = None;
    let mut label = None;
    if policy.checkpoint_before(risk) {
        *generation += 1;
        epoch = Some(*generation);
        let id = world.next_label();
        world.checkpoint(&id)?;
        journal.append("checkpoint", json!({"id": id, "risk": risk.as_str()}))?;
        push_beat(
            goal,
            Mode::Rewind,
            world,
            "checkpoint",
            &id,
            "checkpoint",
            beats,
        )?;
        label = Some(id);
    }
    let before = Digest::from_view(&world.view()?);
    world.apply(&proposal.action)?;
    effects.push(Effect {
        risk,
        generation: epoch,
        undone: false,
    });
    let after = Digest::from_view(&world.view()?);
    journal.append(
        "action",
        json!({
            "fingerprint": proposal.action.fingerprint(),
            "risk": risk.as_str(),
            "tier": tier.as_str(),
            "diff": diff_digests(&before, &after),
            "screenshot_sha256": after.screenshot_sha256,
            "a11y_digest": after.a11y_digest,
        }),
    )?;
    let tone = if risk.is_dangerous() {
        "bad"
    } else {
        "neutral"
    };
    push_beat(
        goal,
        Mode::Rewind,
        world,
        "action",
        &proposal.action.fingerprint(),
        tone,
        beats,
    )?;
    log_observation(journal, world, "after")?;
    let success = goal.satisfied(&world.view()?);
    journal.append("verdict", json!({"ok": success}))?;
    push_beat(
        goal,
        Mode::Rewind,
        world,
        "verdict",
        if success { "ok" } else { "fail" },
        if success { "ok" } else { "bad" },
        beats,
    )?;
    if success {
        return Ok(After::Success);
    }
    if let Some(id) = label {
        world.rewind(&id)?;
        for effect in effects.iter_mut() {
            if effect.generation == epoch && effect.risk != Risk::External {
                effect.undone = true;
            }
        }
        *rewinds += 1;
        journal.append("rewind", json!({"id": id}))?;
        push_beat(goal, Mode::Rewind, world, "rewind", &id, "rewind", beats)?;
        log_observation(journal, world, "restored")?;
        if tier == Tier::Weak && attempts < policy.max_attempts {
            return Ok(After::RetryStrong);
        }
    }
    Ok(After::Failed)
}

fn proposal_payload(proposal: &Proposal, tier: Tier, attempt: u32) -> Value {
    json!({
        "tier": tier.as_str(),
        "attempt": attempt,
        "fingerprint": proposal.action.fingerprint(),
        "confidence": proposal.confidence,
        "agreement": proposal.agreement(),
        "risk": proposal.action.risk().as_str(),
    })
}

fn diff_digests(before: &Digest, after: &Digest) -> Value {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    for (path, hash) in &after.files {
        match before.files.get(path) {
            None => added.push(path.clone()),
            Some(previous) if previous != hash => changed.push(path.clone()),
            Some(_) => {}
        }
    }
    for path in before.files.keys() {
        if !after.files.contains_key(path) {
            removed.push(path.clone());
        }
    }
    json!({"added": added, "removed": removed, "changed": changed})
}

fn log_observation(journal: &mut Journal, world: &DirWorld, phase: &str) -> Result<(), Error> {
    let view = world.view()?;
    let digest = Digest::from_view(&view);
    journal.append(
        "observation",
        json!({
            "phase": phase,
            "files": digest.files,
            "screenshot_sha256": digest.screenshot_sha256,
            "a11y_digest": digest.a11y_digest,
            "outbox": view.outbox,
        }),
    )?;
    Ok(())
}

fn push_beat(
    goal: &Goal,
    mode: Mode,
    world: &DirWorld,
    step: &str,
    detail: &str,
    tone: &str,
    beats: &mut Vec<Beat>,
) -> Result<(), Error> {
    let view = world.view()?;
    beats.push(Beat {
        task_id: goal.id.clone(),
        mode: mode.as_str().to_string(),
        step: step.to_string(),
        detail: detail.to_string(),
        files: view.files.keys().cloned().collect(),
        outbox: view.outbox,
        tone: tone.to_string(),
    });
    Ok(())
}

/// Replay helper used by tests and the CLI.
pub fn replay_kinds(path: &std::path::Path) -> Result<Vec<String>, Error> {
    Ok(journal::replay(path)?
        .into_iter()
        .map(|event| event.kind)
        .collect())
}
