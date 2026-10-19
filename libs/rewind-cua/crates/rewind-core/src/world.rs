//! Workspace the agent acts on.
//!
//! Files live under `root`. Snapshots live beside it, so a rewind cannot
//! delete the snapshots themselves. Mail and payments are recorded in an
//! outbox that rewind does not touch: those effects have already left the
//! machine.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::action::Action;
use crate::snapshot::{self, CommandSnapshot};
use crate::Error;

/// What the verifier and the journal can see.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    /// Relative path to file contents.
    pub files: BTreeMap<String, String>,
    /// External effects, oldest first.
    pub outbox: Vec<String>,
    pub screenshot_sha256: Option<String>,
    pub a11y_digest: Option<String>,
}

/// Directory workspace with either an in-process copy snapshot or a command.
pub struct DirWorld {
    root: PathBuf,
    store: PathBuf,
    outbox: Vec<String>,
    screenshot_sha256: Option<String>,
    a11y_digest: Option<String>,
    backend: Backend,
    next_id: u64,
}

enum Backend {
    Directory,
    Command(CommandSnapshot),
}

impl DirWorld {
    /// Create `workspace/root` and `workspace/snaps`.
    pub fn create(workspace: &Path) -> Result<Self, Error> {
        let root = workspace.join("root");
        let store = workspace.join("snaps");
        fs::create_dir_all(&root).map_err(|err| Error::io(&root, err))?;
        fs::create_dir_all(&store).map_err(|err| Error::io(&store, err))?;
        Ok(Self {
            root,
            store,
            outbox: Vec::new(),
            screenshot_sha256: None,
            a11y_digest: None,
            backend: Backend::Directory,
            next_id: 1,
        })
    }

    /// Same workspace, but checkpoint and rewind shell out.
    pub fn with_command(&mut self, snap: CommandSnapshot) {
        self.backend = Backend::Command(snap);
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn set_desktop_hints(&mut self, screenshot: Option<String>, a11y: Option<String>) {
        self.screenshot_sha256 = screenshot;
        self.a11y_digest = a11y;
    }

    pub fn view(&self) -> Result<View, Error> {
        Ok(View {
            files: read_tree(&self.root, &self.root)?,
            outbox: self.outbox.clone(),
            screenshot_sha256: self.screenshot_sha256.clone(),
            a11y_digest: self.a11y_digest.clone(),
        })
    }

    pub fn file_names(&self) -> Result<Vec<String>, Error> {
        Ok(self.view()?.files.keys().cloned().collect())
    }

    pub fn outbox(&self) -> &[String] {
        &self.outbox
    }

    pub fn apply(&mut self, action: &Action) -> Result<(), Error> {
        match action {
            Action::Noop => Ok(()),
            Action::WriteFile { path, contents } => {
                let dest = resolve(&self.root, path)?;
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).map_err(|err| Error::io(parent, err))?;
                }
                fs::write(&dest, contents).map_err(|err| Error::io(&dest, err))
            }
            Action::DeleteFile { path } => {
                let dest = resolve(&self.root, path)?;
                if !dest.is_file() {
                    return Err(Error::msg(format!("not a file: {path}")));
                }
                fs::remove_file(&dest).map_err(|err| Error::io(&dest, err))
            }
            Action::MoveFile { from, to } => {
                let src = resolve(&self.root, from)?;
                let dest = resolve(&self.root, to)?;
                if !src.is_file() {
                    return Err(Error::msg(format!("not a file: {from}")));
                }
                if dest.exists() {
                    return Err(Error::msg(format!("destination exists: {to}")));
                }
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).map_err(|err| Error::io(parent, err))?;
                }
                fs::rename(&src, &dest).map_err(|err| Error::io(&dest, err))
            }
            Action::SendEmail { to, subject } => {
                self.outbox.push(format!("email:{to}:{subject}"));
                Ok(())
            }
            Action::Pay {
                payee,
                amount_cents,
            } => {
                self.outbox.push(format!("pay:{payee}:{amount_cents}"));
                Ok(())
            }
        }
    }

    pub fn checkpoint(&mut self, label: &str) -> Result<String, Error> {
        validate_label(label)?;
        match &self.backend {
            Backend::Directory => {
                let dest = self.store.join(label);
                if dest.exists() {
                    return Err(Error::msg(format!("checkpoint {label} already exists")));
                }
                snapshot::copy_tree(&self.root, &dest)?;
            }
            Backend::Command(command) => command.capture(label)?,
        }
        self.next_id += 1;
        Ok(label.to_string())
    }

    pub fn rewind(&mut self, label: &str) -> Result<(), Error> {
        validate_label(label)?;
        match &self.backend {
            Backend::Directory => {
                let src = self.store.join(label);
                if !src.is_dir() {
                    return Err(Error::msg(format!("unknown checkpoint {label}")));
                }
                snapshot::clear_dir(&self.root)?;
                snapshot::copy_tree(&src, &self.root)?;
            }
            Backend::Command(command) => command.restore(label)?,
        }
        Ok(())
    }

    pub fn next_label(&self) -> String {
        format!("ck-{:04}", self.next_id)
    }
}

fn validate_label(label: &str) -> Result<(), Error> {
    if label.is_empty()
        || label.len() > 64
        || !label
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err(Error::msg(format!("bad checkpoint label {label}")));
    }
    Ok(())
}

fn resolve(root: &Path, rel: &str) -> Result<PathBuf, Error> {
    if rel.is_empty() || rel.len() > 200 {
        return Err(Error::msg(format!("path is empty or too long: {rel}")));
    }
    if rel.starts_with('/') || rel.contains('\\') || rel.contains('\0') {
        return Err(Error::msg(format!(
            "path must stay inside the workspace: {rel}"
        )));
    }
    let mut out = root.to_path_buf();
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => {
                return Err(Error::msg(format!(
                    "path must stay inside the workspace: {rel}"
                )))
            }
        }
    }
    Ok(out)
}

fn read_tree(root: &Path, dir: &Path) -> Result<BTreeMap<String, String>, Error> {
    let mut files = BTreeMap::new();
    let entries = fs::read_dir(dir).map_err(|err| Error::io(dir, err))?;
    for entry in entries {
        let entry = entry.map_err(|err| Error::io(dir, err))?;
        let file_type = entry
            .file_type()
            .map_err(|err| Error::io(&entry.path(), err))?;
        if file_type.is_symlink() {
            return Err(Error::msg(format!(
                "refusing to read symlink {}",
                entry.path().display()
            )));
        }
        if file_type.is_dir() {
            files.extend(read_tree(root, &entry.path())?);
        } else if file_type.is_file() {
            let rel = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| Error::msg("file escaped the workspace"))?
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let text =
                fs::read_to_string(entry.path()).map_err(|err| Error::io(&entry.path(), err))?;
            files.insert(rel, text);
        }
    }
    Ok(files)
}

/// File hashes for the journal. Contents stay in the checkpoint, not the log.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Digest {
    pub files: BTreeMap<String, String>,
    pub screenshot_sha256: Option<String>,
    pub a11y_digest: Option<String>,
}

impl Digest {
    pub fn from_view(view: &View) -> Self {
        let files = view
            .files
            .iter()
            .map(|(path, contents)| {
                (
                    path.clone(),
                    crate::journal::sha256_hex(contents.as_bytes()),
                )
            })
            .collect();
        Self {
            files,
            screenshot_sha256: view.screenshot_sha256.clone(),
            a11y_digest: view.a11y_digest.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_restores_a_deleted_nested_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = DirWorld::create(dir.path()).unwrap();
        world
            .apply(&Action::WriteFile {
                path: "archive/notes.txt".into(),
                contents: "scratch\n".into(),
            })
            .unwrap();
        let label = world.next_label();
        world.checkpoint(&label).unwrap();
        world
            .apply(&Action::DeleteFile {
                path: "archive/notes.txt".into(),
            })
            .unwrap();
        assert!(world.view().unwrap().files.is_empty());
        world.rewind(&label).unwrap();
        assert_eq!(
            world
                .view()
                .unwrap()
                .files
                .get("archive/notes.txt")
                .unwrap(),
            "scratch\n"
        );
    }

    #[test]
    fn rewind_does_not_unsend_mail() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = DirWorld::create(dir.path()).unwrap();
        world
            .apply(&Action::WriteFile {
                path: "draft.txt".into(),
                contents: "hello\n".into(),
            })
            .unwrap();
        let label = world.next_label();
        world.checkpoint(&label).unwrap();
        world
            .apply(&Action::SendEmail {
                to: "ada@example.com".into(),
                subject: "hi".into(),
            })
            .unwrap();
        world
            .apply(&Action::DeleteFile {
                path: "draft.txt".into(),
            })
            .unwrap();
        world.rewind(&label).unwrap();
        let view = world.view().unwrap();
        assert_eq!(view.files.get("draft.txt").unwrap(), "hello\n");
        assert_eq!(view.outbox, vec!["email:ada@example.com:hi".to_string()]);
    }

    #[test]
    fn parent_paths_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = DirWorld::create(dir.path()).unwrap();
        let err = world
            .apply(&Action::DeleteFile {
                path: "../snaps".into(),
            })
            .unwrap_err();
        assert!(err.to_string().contains("workspace"), "{err}");
    }
}
