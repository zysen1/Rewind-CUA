//! Append-only action journal.
//!
//! Each line is one JSON object. The `hash` field is SHA-256 over the rest of
//! the object, and that object includes the previous hash, so a torn or edited
//! line breaks replay. A sidecar `{path}.tip` stores the latest hash so
//! truncating the log is visible the next time it is opened.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::Error;

/// 64 hex zeros. The hash before the first record.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One verified journal record.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub seq: u64,
    pub ts: u64,
    pub kind: String,
    pub payload: Value,
    pub hash: String,
}

/// Append-only journal file plus its tip sidecar.
pub struct Journal {
    path: PathBuf,
    tip_path: PathBuf,
    file: File,
    seq: u64,
    last_hash: String,
    clock: u64,
}

#[derive(Serialize)]
struct Body<'a> {
    seq: u64,
    ts: u64,
    kind: &'a str,
    prev: &'a str,
    payload: &'a Value,
}

#[derive(Serialize)]
struct Line<'a> {
    seq: u64,
    ts: u64,
    kind: &'a str,
    prev: &'a str,
    payload: &'a Value,
    hash: &'a str,
}

impl Journal {
    /// Create a new empty journal. Fails if `path` already exists.
    pub fn create(path: &Path) -> Result<Self, Error> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| Error::io(parent, err))?;
        }
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(path)
            .map_err(|err| Error::io(path, err))?;
        let journal = Self {
            path: path.to_path_buf(),
            tip_path: tip_path(path),
            file,
            seq: 0,
            last_hash: GENESIS.to_string(),
            clock: 0,
        };
        journal.write_tip()?;
        Ok(journal)
    }

    /// Open an existing journal and verify the chain and the tip.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let events = replay(path)?;
        let file = OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|err| Error::io(path, err))?;
        let (seq, last_hash, clock) = match events.last() {
            Some(event) => (event.seq, event.hash.clone(), event.ts),
            None => (0, GENESIS.to_string(), 0),
        };
        Ok(Self {
            path: path.to_path_buf(),
            tip_path: tip_path(path),
            file,
            seq,
            last_hash,
            clock,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Append one record. `ts` advances by one on each call so a run is reproducible.
    pub fn append(&mut self, kind: &str, payload: Value) -> Result<Event, Error> {
        self.seq += 1;
        self.clock += 1;
        let body = Body {
            seq: self.seq,
            ts: self.clock,
            kind,
            prev: &self.last_hash,
            payload: &payload,
        };
        let bytes = serde_json::to_vec(&body)?;
        let hash = sha256_hex(&bytes);
        let line = Line {
            seq: self.seq,
            ts: self.clock,
            kind,
            prev: &self.last_hash,
            payload: &payload,
            hash: &hash,
        };
        let encoded = serde_json::to_string(&line)?;
        self.file
            .write_all(encoded.as_bytes())
            .map_err(|err| Error::io(&self.path, err))?;
        self.file
            .write_all(b"\n")
            .map_err(|err| Error::io(&self.path, err))?;
        self.file
            .sync_all()
            .map_err(|err| Error::io(&self.path, err))?;
        self.last_hash = hash.clone();
        self.write_tip()?;
        Ok(Event {
            seq: self.seq,
            ts: self.clock,
            kind: kind.to_string(),
            payload,
            hash,
        })
    }

    fn write_tip(&self) -> Result<(), Error> {
        let body = format!("{} {}\n", self.seq, self.last_hash);
        fs::write(&self.tip_path, body).map_err(|err| Error::io(&self.tip_path, err))?;
        // Re-open to sync. `fs::write` already syncs on close on most platforms;
        // open and sync so a crash cannot leave a stale tip after a durable line.
        let tip = OpenOptions::new()
            .write(true)
            .open(&self.tip_path)
            .map_err(|err| Error::io(&self.tip_path, err))?;
        tip.sync_all()
            .map_err(|err| Error::io(&self.tip_path, err))?;
        Ok(())
    }
}

/// Read `path`, verify every hash, and check the tip sidecar.
pub fn replay(path: &Path) -> Result<Vec<Event>, Error> {
    let file = File::open(path).map_err(|err| Error::io(path, err))?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    let mut prev = GENESIS.to_string();
    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(|err| Error::io(path, err))?;
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line).map_err(|err| {
            Error::msg(format!(
                "{}:{}: not a journal record ({err})",
                path.display(),
                index + 1
            ))
        })?;
        let seq = value
            .get("seq")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::msg(format!("{}:{}: missing seq", path.display(), index + 1)))?;
        let ts = value
            .get("ts")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::msg(format!("{}:{}: missing ts", path.display(), index + 1)))?;
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::msg(format!("{}:{}: missing kind", path.display(), index + 1)))?
            .to_string();
        let prev_field = value
            .get("prev")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::msg(format!("{}:{}: missing prev", path.display(), index + 1)))?;
        let payload = value.get("payload").cloned().unwrap_or(json!({}));
        let hash = value
            .get("hash")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::msg(format!("{}:{}: missing hash", path.display(), index + 1)))?
            .to_string();
        if prev_field != prev {
            return Err(Error::msg(format!(
                "{}:{}: prev hash does not continue the chain",
                path.display(),
                index + 1
            )));
        }
        if seq != events.len() as u64 + 1 {
            return Err(Error::msg(format!(
                "{}:{}: expected seq {}",
                path.display(),
                index + 1,
                events.len() + 1
            )));
        }
        let body = Body {
            seq,
            ts,
            kind: &kind,
            prev: prev_field,
            payload: &payload,
        };
        let bytes = serde_json::to_vec(&body)?;
        let expect = sha256_hex(&bytes);
        if expect != hash {
            return Err(Error::msg(format!(
                "{}:{}: hash mismatch (record was edited)",
                path.display(),
                index + 1
            )));
        }
        prev = hash.clone();
        events.push(Event {
            seq,
            ts,
            kind,
            payload,
            hash,
        });
    }
    check_tip(path, events.len() as u64, &prev)?;
    Ok(events)
}

fn check_tip(path: &Path, seq: u64, hash: &str) -> Result<(), Error> {
    let tip = tip_path(path);
    let text = fs::read_to_string(&tip).map_err(|err| Error::io(&tip, err))?;
    let mut parts = text.split_whitespace();
    let tip_seq = parts
        .next()
        .ok_or_else(|| Error::msg(format!("{}: empty tip", tip.display())))?;
    let tip_hash = parts
        .next()
        .ok_or_else(|| Error::msg(format!("{}: tip has no hash", tip.display())))?;
    let tip_seq: u64 = tip_seq
        .parse()
        .map_err(|_| Error::msg(format!("{}: tip seq is not an integer", tip.display())))?;
    if tip_seq != seq || tip_hash != hash {
        return Err(Error::msg(format!(
            "{}: tip is {tip_seq} {tip_hash}, log ends at {seq} {hash} (truncated or crashed mid-append)",
            tip.display()
        )));
    }
    Ok(())
}

fn tip_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".tip");
    PathBuf::from(name)
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    const HEX: &[u8] = b"0123456789abcdef";
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn append_replay_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let mut journal = Journal::create(&path).unwrap();
        journal
            .append("observation", json!({"files": ["a"]}))
            .unwrap();
        journal
            .append("action", json!({"fingerprint": "noop"}))
            .unwrap();
        let events = replay(&path).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "observation");
        assert_eq!(events[1].seq, 2);
        assert_ne!(events[0].hash, events[1].hash);

        let mut again = Journal::open(&path).unwrap();
        again.append("verdict", json!({"ok": true})).unwrap();
        let events = replay(&path).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[2].kind, "verdict");
    }

    #[test]
    fn edited_line_breaks_the_chain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let mut journal = Journal::create(&path).unwrap();
        journal
            .append("action", json!({"fingerprint": "noop"}))
            .unwrap();
        let text = fs::read_to_string(&path).unwrap().replace("noop", "no-op");
        fs::write(&path, text).unwrap();
        let err = replay(&path).unwrap_err();
        assert!(err.to_string().contains("hash mismatch"), "{err}");
    }

    #[test]
    fn truncating_the_log_breaks_the_tip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let mut journal = Journal::create(&path).unwrap();
        journal.append("action", json!({"n": 1})).unwrap();
        journal.append("action", json!({"n": 2})).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let first = text.lines().next().unwrap();
        fs::write(&path, format!("{first}\n")).unwrap();
        let err = replay(&path).unwrap_err();
        assert!(err.to_string().contains("tip"), "{err}");
    }
}
