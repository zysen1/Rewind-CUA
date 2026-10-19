//! `rewind`: run the scripted eval, the demo, or replay a journal.

#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use rewind_core::demo::bundled_storyboard;
use rewind_core::eval::{self, load_bundled};
use rewind_core::journal;

#[derive(Parser)]
#[command(
    name = "rewind",
    version,
    about = "Rewind-CUA: journal, policy, checkpoint, and rewind"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the scripted vanilla versus Rewind-CUA comparison.
    Eval {
        /// Write the JSON report here. Prints it when omitted.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run the mistaken-delete demo and write a storyboard JSON.
    Demo {
        /// Directory for storyboard.json.
        #[arg(long)]
        out: PathBuf,
    },
    /// Verify an action journal and print its records.
    Replay { path: PathBuf },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Eval { out } => cmd_eval(out),
        Command::Demo { out } => cmd_demo(out),
        Command::Replay { path } => cmd_replay(&path),
    }
}

fn cmd_eval(out: Option<PathBuf>) -> ExitCode {
    let scratch = std::env::temp_dir().join(format!("rewind-cua-eval-{}", std::process::id()));
    if let Err(err) = fs::create_dir_all(&scratch) {
        eprintln!("rewind: {err}");
        return ExitCode::from(1);
    }
    let (policy, tasks) = match load_bundled() {
        Ok(loaded) => loaded,
        Err(err) => {
            eprintln!("rewind: {err}");
            return ExitCode::from(1);
        }
    };
    let run = match eval::run_suite(&scratch, &tasks, &policy) {
        Ok(run) => run,
        Err(err) => {
            eprintln!("rewind: {err}");
            return ExitCode::from(1);
        }
    };
    let json = match serde_json::to_string_pretty(&run.report) {
        Ok(json) => json,
        Err(err) => {
            eprintln!("rewind: {err}");
            return ExitCode::from(1);
        }
    };
    print_scores(&run.report);
    if let Some(path) = out {
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                eprintln!("rewind: {err}");
                return ExitCode::from(1);
            }
        }
        if let Err(err) = fs::write(&path, format!("{json}\n")) {
            eprintln!("rewind: {err}");
            return ExitCode::from(1);
        }
        println!("wrote {}", path.display());
    } else {
        println!("{json}");
    }
    ExitCode::SUCCESS
}

fn cmd_demo(out: PathBuf) -> ExitCode {
    let scratch = out.join("scratch");
    if let Err(err) = fs::create_dir_all(&scratch) {
        eprintln!("rewind: {err}");
        return ExitCode::from(1);
    }
    let (report, board) = match bundled_storyboard(&scratch) {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("rewind: {err}");
            return ExitCode::from(1);
        }
    };
    let json = match serde_json::to_string_pretty(&board) {
        Ok(json) => json,
        Err(err) => {
            eprintln!("rewind: {err}");
            return ExitCode::from(1);
        }
    };
    if let Err(err) = fs::create_dir_all(&out) {
        eprintln!("rewind: {err}");
        return ExitCode::from(1);
    }
    let path = out.join("storyboard.json");
    if let Err(err) = fs::write(&path, format!("{json}\n")) {
        eprintln!("rewind: {err}");
        return ExitCode::from(1);
    }
    print_scores(&report);
    println!("wrote {}", path.display());
    for frame in &board.frames {
        println!("[{}] {} | {}", frame.kicker, frame.title_en, frame.title_zh);
    }
    ExitCode::SUCCESS
}

fn cmd_replay(path: &std::path::Path) -> ExitCode {
    match journal::replay(path) {
        Ok(events) => {
            for event in events {
                println!("{:04} {:<14} {}", event.seq, event.kind, event.payload);
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("rewind: {err}");
            ExitCode::from(1)
        }
    }
}

fn print_scores(report: &eval::SuiteReport) {
    println!(
        "vanilla {}/{} success, {} irreversible stuck",
        report.vanilla.successes, report.tasks, report.vanilla.irreversible_errors
    );
    println!(
        "rewind  {}/{} success, {} irreversible stuck, {} rewinds, {} escalations, {} human prompts, {} policy denies",
        report.rewind.successes,
        report.tasks,
        report.rewind.irreversible_errors,
        report.rewind.rewinds,
        report.rewind.uncertainty_escalations,
        report.rewind.human_prompts,
        report.rewind.policy_denies
    );
}
