//! Storyboard for the mistaken-delete demo.
//!
//! Frames are taken from real episode beats, then given bilingual captions.

use serde::{Deserialize, Serialize};

use crate::eval::{SuiteReport, SuiteRun};
use crate::harness::Beat;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub tone: String,
    pub kicker: String,
    pub title_en: String,
    pub title_zh: String,
    pub detail_en: String,
    pub detail_zh: String,
    pub files: Vec<String>,
    pub outbox: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Storyboard {
    pub schema_version: u32,
    pub title: String,
    pub frames: Vec<Frame>,
}

pub fn storyboard(run: &SuiteRun) -> Storyboard {
    let mut frames = vec![Frame {
        tone: "neutral".into(),
        kicker: "REWIND-CUA".into(),
        title_en: "The agent deletes the report.".into(),
        title_zh: "智能体误删了报告。".into(),
        detail_en: "Rewind-CUA checkpoints first, then puts the file back.".into(),
        detail_zh: "Rewind-CUA 先打检查点，再把文件倒带回来。".into(),
        files: Vec::new(),
        outbox: Vec::new(),
    }];
    for episode in &run.episodes {
        if !keep(episode.task_id.as_str(), episode.mode.as_str()) {
            continue;
        }
        for beat in &episode.beats {
            frames.push(frame_from_beat(beat));
        }
    }
    let report = &run.report;
    frames.push(Frame {
        tone: "ok".into(),
        kicker: "MEASURED".into(),
        title_en: format!(
            "Scripted tasks: Rewind-CUA {}/{}, vanilla {}/{}",
            report.rewind.successes, report.tasks, report.vanilla.successes, report.tasks
        ),
        title_zh: format!(
            "脚本任务：Rewind-CUA {}/{}，普通执行 {}/{}",
            report.rewind.successes, report.tasks, report.vanilla.successes, report.tasks
        ),
        detail_en: format!(
            "Irreversible mistakes that stuck: rewind {}, vanilla {}. Cua-Bench and OSWorld: WIP.",
            report.rewind.irreversible_errors, report.vanilla.irreversible_errors
        ),
        detail_zh: format!(
            "留下的不可逆错误：Rewind {}，普通执行 {}。Cua-Bench 与 OSWorld：未跑。",
            report.rewind.irreversible_errors, report.vanilla.irreversible_errors
        ),
        files: Vec::new(),
        outbox: Vec::new(),
    });
    Storyboard {
        schema_version: 1,
        title: "Rewind-CUA mistaken delete".into(),
        frames,
    }
}

fn keep(task: &str, mode: &str) -> bool {
    task == "mistaken-delete" || (task == "approval-send" && mode == "rewind")
}

fn frame_from_beat(beat: &Beat) -> Frame {
    let (title_en, title_zh, detail_en, detail_zh) =
        captions(&beat.task_id, &beat.step, &beat.detail);
    Frame {
        tone: beat.tone.clone(),
        kicker: format!("{}  {}", beat.mode, beat.task_id),
        title_en,
        title_zh,
        detail_en,
        detail_zh,
        files: beat.files.clone(),
        outbox: beat.outbox.clone(),
    }
}

fn captions(task: &str, step: &str, detail: &str) -> (String, String, String, String) {
    let pair = match (step, detail) {
        ("start", _) if task == "mistaken-delete" => (
            "Workspace before the agent acts",
            "智能体动手之前",
            "report.txt and notes.txt are both here.",
            "report.txt 和 notes.txt 都还在。",
        ),
        ("start", _) => (
            "Draft is still unsent",
            "草稿还没发出去",
            "The next action wants to send mail.",
            "下一步想把邮件发出去。",
        ),
        ("checkpoint", _) => (
            "Checkpoint saved",
            "检查点已保存",
            "The workspace can be restored from here.",
            "工作区可以从这里恢复。",
        ),
        ("action", detail) if detail.starts_with("delete_file:") => (
            "The executor deleted report.txt",
            "执行模型删掉了 report.txt",
            "It was sure, and it deleted the wrong file.",
            "它很有把握，但删错了文件。",
        ),
        ("action", detail) if detail.starts_with("move_file:") => (
            "The stronger model archived the notes",
            "更强的模型把笔记归档了",
            "report.txt is still there.",
            "report.txt 还在。",
        ),
        ("action", "noop") => (
            "Nothing was sent",
            "什么都没发出去",
            "The stronger model left the draft alone.",
            "更强的模型没有动这份草稿。",
        ),
        ("verdict", "fail") => (
            "Verifier rejected the result",
            "校验没有通过",
            "The report is gone, so the task failed.",
            "报告没了，任务失败。",
        ),
        ("verdict", "ok") if task == "approval-send" => (
            "The draft stayed on disk",
            "草稿还在磁盘上",
            "A person said no, so the mail was not sent.",
            "人拒绝了，邮件没有发出。",
        ),
        ("verdict", "ok") => (
            "Task succeeded after rewind",
            "倒带之后任务成功",
            "The report is back, and the notes are archived.",
            "报告回来了，笔记也归档了。",
        ),
        ("rewind", _) => (
            "Rewound to the checkpoint",
            "已回滚到检查点",
            "report.txt is restored. The delete did not stick.",
            "report.txt 已恢复。这次删除没有留下来。",
        ),
        ("approval", detail) if detail.starts_with("ask:") => (
            "Approval required",
            "需要人工批准",
            "Outbound mail waits for a person.",
            "对外发信要等人点头。",
        ),
        ("approval", _) => (
            "The person said no",
            "人拒绝了",
            "The harness asks the stronger model for a safer step.",
            "Harness 改向更强的模型要一个更安全的步骤。",
        ),
        _ => (step, step, detail, detail),
    };
    (
        pair.0.to_string(),
        pair.1.to_string(),
        pair.2.to_string(),
        pair.3.to_string(),
    )
}

/// Convenience for the CLI: run the bundled suite and build the storyboard.
pub fn bundled_storyboard(
    scratch: &std::path::Path,
) -> Result<(SuiteReport, Storyboard), crate::Error> {
    let (policy, tasks) = crate::eval::load_bundled()?;
    let run = crate::eval::run_suite(scratch, &tasks, &policy)?;
    let board = storyboard(&run);
    Ok((run.report, board))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::{bundled_root, load_bundled, run_suite};
    use std::fs;

    #[test]
    fn storyboard_shows_rewind_and_matches_the_committed_file() {
        let (policy, tasks) = load_bundled().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let run = run_suite(scratch.path(), &tasks, &policy).unwrap();
        let board = storyboard(&run);
        assert!(board.frames.iter().any(|frame| frame.tone == "rewind"));
        assert!(board.frames.iter().any(|frame| frame.tone == "ask"));
        let restored = board
            .frames
            .iter()
            .find(|frame| frame.tone == "rewind")
            .unwrap();
        assert!(restored.files.iter().any(|file| file == "report.txt"));
        let committed = bundled_root().join("demo/storyboard.json");
        let text = fs::read_to_string(&committed).unwrap();
        let parsed: Storyboard = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, board);
    }
}
