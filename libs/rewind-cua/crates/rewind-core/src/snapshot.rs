//! Snapshot backends.
//!
//! The directory backend is what the demo and the eval run. It copies the
//! workspace tree. The command backend runs a caller-supplied capture and
//! restore program so a real VM checkpoint can be plugged in without this
//! crate talking to QEMU or Lume itself.
//!
//! What upstream trycua/cua actually offers is documented in
//! [`documented_backends`] and in `docs/SNAPSHOTS.md`. `Sandbox.snapshot()`
//! raises `CloudEnded`. Named rewind points that do exist are QEMU `savevm` /
//! `loadvm` and Lume's stop-clone-restart checkpoint.

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::Error;

/// A named snapshot backend a human can point at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendDoc {
    pub id: &'static str,
    /// This process invokes the backend itself.
    pub invoked_by_default: bool,
    pub summary: &'static str,
}

/// Backends Rewind-CUA knows about. Only `directory` and `command` run here.
pub fn documented_backends() -> &'static [BackendDoc] {
    &[
        BackendDoc {
            id: "directory",
            invoked_by_default: true,
            summary: "Copy the workspace directory before a risky action and copy it back on rewind. Used by the demo and the scripted eval. Does not roll back a guest OS.",
        },
        BackendDoc {
            id: "command",
            invoked_by_default: false,
            summary: "Run a caller-supplied capture command and restore command. Tests use a shell script. A VM wrapper would call the QEMU or Lume operations below.",
        },
        BackendDoc {
            id: "qemu-savevm",
            invoked_by_default: false,
            summary: "Upstream libs/cua/crates/cua-vmm QEMU Runtime::checkpoint issues QMP savevm while the VM is running (RAM, devices, and disks inside the qcow2). Runtime::restore issues loadvm. A stopped VM freezes the disk into an immutable layer instead. suspend is QMP stop, a pause, not a named checkpoint. The Python bare-metal runtime saves one slot named cua-snapshot and resumes with -loadvm.",
        },
        BackendDoc {
            id: "lume-clone",
            invoked_by_default: false,
            summary: "Upstream Lume Runtime::checkpoint stops a running VM, clones it, and starts it again if it was running. There is no in-memory pause: suspend stops the VM. Linux disk clones try cp -c (APFS clonefile) and fall back to a full copy.",
        },
        BackendDoc {
            id: "sandbox-suspend",
            invoked_by_default: false,
            summary: "Python Sandbox.suspend / resume is one power transition (container pause, QEMU pause, or Lume stop). Sandbox.snapshot() raises CloudEnded. Neither is a stack of named rewind points.",
        },
    ]
}

/// External capture and restore commands.
///
/// Arguments may contain `{label}`, `{root}`, and `{store}`.
#[derive(Clone, Debug)]
pub struct CommandSnapshot {
    pub program: String,
    pub capture_args: Vec<String>,
    pub restore_args: Vec<String>,
    pub root: std::path::PathBuf,
    pub store: std::path::PathBuf,
}

impl CommandSnapshot {
    pub fn capture(&self, label: &str) -> Result<(), Error> {
        self.run(&self.capture_args, label)
    }

    pub fn restore(&self, label: &str) -> Result<(), Error> {
        self.run(&self.restore_args, label)
    }

    fn run(&self, args: &[String], label: &str) -> Result<(), Error> {
        let args: Vec<String> = args.iter().map(|arg| self.expand(arg, label)).collect();
        let output = Command::new(&self.program)
            .args(&args)
            .output()
            .map_err(|err| Error::msg(format!("snapshot command {}: {err}", self.program)))?;
        if !output.status.success() {
            return Err(Error::msg(format!(
                "snapshot command failed ({})",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(())
    }

    fn expand(&self, arg: &str, label: &str) -> String {
        arg.replace("{label}", label)
            .replace("{root}", &self.root.display().to_string())
            .replace("{store}", &self.store.display().to_string())
    }
}

pub(crate) fn copy_tree(src: &Path, dst: &Path) -> Result<(), Error> {
    fs::create_dir_all(dst).map_err(|err| Error::io(dst, err))?;
    let entries = fs::read_dir(src).map_err(|err| Error::io(src, err))?;
    for entry in entries {
        let entry = entry.map_err(|err| Error::io(src, err))?;
        let file_type = entry
            .file_type()
            .map_err(|err| Error::io(&entry.path(), err))?;
        if file_type.is_symlink() {
            return Err(Error::msg(format!(
                "refusing to snapshot symlink {}",
                entry.path().display()
            )));
        }
        let to = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &to).map_err(|err| Error::io(&to, err))?;
        } else {
            return Err(Error::msg(format!(
                "unsupported file type {}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

pub(crate) fn clear_dir(dir: &Path) -> Result<(), Error> {
    if !dir.exists() {
        fs::create_dir_all(dir).map_err(|err| Error::io(dir, err))?;
        return Ok(());
    }
    let entries = fs::read_dir(dir).map_err(|err| Error::io(dir, err))?;
    for entry in entries {
        let entry = entry.map_err(|err| Error::io(dir, err))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|err| Error::io(&path, err))?;
        if file_type.is_symlink() || file_type.is_file() {
            fs::remove_file(&path).map_err(|err| Error::io(&path, err))?;
        } else if file_type.is_dir() {
            fs::remove_dir_all(&path).map_err(|err| Error::io(&path, err))?;
        } else {
            return Err(Error::msg(format!(
                "unsupported file type {}",
                path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_backends_name_the_upstream_mechanisms() {
        let docs = documented_backends();
        let qemu = docs.iter().find(|doc| doc.id == "qemu-savevm").unwrap();
        assert!(!qemu.invoked_by_default);
        assert!(qemu.summary.contains("savevm"));
        assert!(qemu.summary.contains("loadvm"));
        let lume = docs.iter().find(|doc| doc.id == "lume-clone").unwrap();
        assert!(lume.summary.contains("Runtime::checkpoint"));
        let cloud = docs.iter().find(|doc| doc.id == "sandbox-suspend").unwrap();
        assert!(cloud.summary.contains("CloudEnded"));
        assert!(docs
            .iter()
            .any(|doc| doc.id == "directory" && doc.invoked_by_default));
    }

    #[test]
    fn command_backend_restores_a_deleted_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let store = dir.path().join("store");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("report.txt"), "Q3 report\n").unwrap();
        let script = dir.path().join("snap.sh");
        fs::write(
            &script,
            "#!/bin/sh\nset -eu\nop=\"$1\"\nlabel=\"$2\"\nroot=\"$3\"\nstore=\"$4\"\nif [ \"$op\" = capture ]; then\n  rm -rf \"$store/$label\"\n  mkdir -p \"$store/$label\"\n  cp -a \"$root/.\" \"$store/$label/\"\nelse\n  find \"$root\" -mindepth 1 -exec rm -rf {} +\n  mkdir -p \"$root\"\n  cp -a \"$store/$label/.\" \"$root/\"\nfi\n",
        )
        .unwrap();
        let snap = CommandSnapshot {
            program: "sh".into(),
            capture_args: vec![
                script.display().to_string(),
                "capture".into(),
                "{label}".into(),
                "{root}".into(),
                "{store}".into(),
            ],
            restore_args: vec![
                script.display().to_string(),
                "restore".into(),
                "{label}".into(),
                "{root}".into(),
                "{store}".into(),
            ],
            root: root.clone(),
            store,
        };
        snap.capture("ck-1").unwrap();
        fs::remove_file(root.join("report.txt")).unwrap();
        assert!(!root.join("report.txt").exists());
        snap.restore("ck-1").unwrap();
        let text = fs::read_to_string(root.join("report.txt")).unwrap();
        assert_eq!(text, "Q3 report\n");
    }
}
