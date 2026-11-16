//! Bounded, incremental JSONL reads; tolerate partial appends and file replacement.
use serde_json::Value;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};
const READ_BUDGET: u64 = 1024 * 1024;

#[derive(Default)]
pub(crate) struct JsonlCursor {
    path: PathBuf,
    offset: u64,
    modified: Option<SystemTime>,
    pending: Vec<u8>,
    dropping_long_line: bool,
}
impl JsonlCursor {
    pub(crate) fn caught_up(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|meta| self.offset >= meta.len())
            && self.pending.is_empty()
    }
    pub(crate) fn read(&mut self, path: &Path) -> (bool, Vec<Value>) {
        let Ok(mut file) = File::open(path) else {
            return (false, vec![]);
        };
        let Ok(meta) = file.metadata() else {
            return (false, vec![]);
        };
        let modified = meta.modified().ok();
        let reset = self.path != path
            || meta.len() < self.offset
            || (meta.len() == self.offset && modified != self.modified);
        if reset {
            self.path = path.to_owned();
            self.offset = 0;
            self.pending.clear();
            self.dropping_long_line = false;
        }
        self.modified = modified;
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return (reset, vec![]);
        }
        let mut bytes = Vec::new();
        if file.take(READ_BUDGET).read_to_end(&mut bytes).is_err() {
            return (reset, vec![]);
        }
        self.offset += bytes.len() as u64;
        let mut records = Vec::new();
        for part in bytes.split_inclusive(|byte| *byte == b'\n') {
            let complete = part.ends_with(b"\n");
            if !self.dropping_long_line {
                if self.pending.len() + part.len() <= READ_BUDGET as usize {
                    self.pending.extend_from_slice(part);
                    if complete && let Ok(value) = serde_json::from_slice(&self.pending) {
                        records.push(value);
                    }
                } else {
                    self.dropping_long_line = true;
                    self.pending.clear();
                }
            }
            if complete {
                self.pending.clear();
                self.dropping_long_line = false;
            }
        }
        (reset, records)
    }
}
