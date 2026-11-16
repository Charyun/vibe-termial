//! Agent turn state comes from OSC 7501, with session-log fallback for older CLIs.
use crate::{
    conversation::ConversationKey,
    jsonl::JsonlCursor,
    model::{CliKind, Tab},
    transcript::{self, Role},
};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use uuid::Uuid;
pub const QUERY_REPLY: &[u8] =
    b"\x1b]7501;?:states=idle,working,blocked,done,error:kinds=permission,question,auth\x1b\\";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Working,
    Blocked,
    Done,
    Error,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Indicator {
    #[default]
    None,
    Working,
    Blocked,
    Completed,
    Failed,
}
#[derive(Default)]
pub struct Activity {
    native: bool,
    managed: bool,
    cancelled: bool,
    root: Phase,
    children: HashMap<String, Phase>,
    attention: Option<Indicator>,
}
impl Activity {
    pub fn native(&self) -> bool {
        self.native
    }
    pub fn indicator(&self, running: bool) -> Indicator {
        match self.effective() {
            Phase::Working if running => Indicator::Working,
            Phase::Blocked if running => Indicator::Blocked,
            _ => self.attention.unwrap_or_default(),
        }
    }
    fn effective(&self) -> Phase {
        if self
            .children
            .values()
            .any(|phase| matches!(phase, Phase::Working | Phase::Blocked))
        {
            if self.children.values().any(|phase| *phase == Phase::Working) {
                Phase::Working
            } else {
                Phase::Blocked
            }
        } else {
            self.root
        }
    }
    fn advance(&mut self, before: Phase, seed: bool) {
        let next = self.effective();
        if seed || matches!(next, Phase::Working | Phase::Blocked) {
            self.attention = None;
        } else if matches!(before, Phase::Working | Phase::Blocked) {
            self.attention = match next {
                Phase::Done => Some(Indicator::Completed),
                Phase::Error => Some(Indicator::Failed),
                _ => self.attention,
            };
        }
    }
    /// Ignore child-only completions, repeated done records and initial historical results.
    pub fn osc(&mut self, body: &str) -> bool {
        if body.len() > 4087 {
            return false;
        }
        let mut state = None;
        let mut id = None;
        for pair in body.split(':') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            if key.len() > 16 {
                return false;
            }
            match key {
                "state" => state = Some(value),
                "id" => {
                    if value.is_empty()
                        || value.len() > 128
                        || value.split('/').count() > 8
                        || value.split('/').any(|part| {
                            part.is_empty()
                                || part.len() > 32
                                || !part.bytes().all(|byte| {
                                    byte.is_ascii_alphanumeric() || b"_.+-".contains(&byte)
                                })
                        })
                    {
                        return false;
                    }
                    id = Some(value);
                }
                _ => {}
            }
        }
        let phase = match state {
            Some("idle") => Phase::Idle,
            Some("working") => Phase::Working,
            Some("blocked") => Phase::Blocked,
            Some("done") => Phase::Done,
            Some("error") => Phase::Error,
            Some("clear") => Phase::Idle,
            _ => return false,
        };
        let before = self.effective();
        self.native = true;
        if let Some(id) = id {
            if matches!(phase, Phase::Working | Phase::Blocked) {
                if self.children.len() < 128 || self.children.contains_key(id) {
                    self.children.insert(id.into(), phase);
                }
            } else {
                self.children.remove(id);
            }
        } else {
            self.root = phase;
            if state == Some("clear") {
                self.children.clear();
            }
        }
        self.advance(before, false);
        true
    }
    pub fn submitted(&mut self) {
        if self.native {
            return;
        }
        let old = self.effective();
        self.managed = true;
        self.cancelled = false;
        self.root = Phase::Working;
        self.advance(old, false);
    }
    pub fn interrupted(&mut self) {
        if self.native || !matches!(self.effective(), Phase::Working | Phase::Blocked) {
            return;
        }
        self.managed = true;
        self.cancelled = true;
        self.root = Phase::Idle;
        self.attention = None;
    }
    pub fn managed_completed(&mut self) {
        if self.native || self.cancelled {
            return;
        }
        self.managed = true;
        self.root = Phase::Done;
        self.attention = Some(Indicator::Completed);
    }
    pub fn fallback(&mut self, phase: Phase, seed: bool) -> bool {
        if self.native || (self.managed && (seed || !matches!(phase, Phase::Error | Phase::Idle))) {
            return false;
        }
        let before = self.indicator(true);
        let old = self.effective();
        self.root = phase;
        self.advance(old, seed);
        before != self.indicator(true)
    }
    pub fn acknowledge(&mut self) {
        self.attention = None;
    }
}

pub struct Update {
    pub key: ConversationKey,
    pub phase: Phase,
    pub seed: bool,
    pub worked: bool,
}
struct LogState {
    key: ConversationKey,
    source: Option<PathBuf>,
    cursor: JsonlCursor,
    phase: Phase,
    seeding: bool,
}
#[derive(Default)]
pub struct ActivityReader {
    cached: HashMap<Uuid, LogState>,
    completions: HashMap<(Uuid, Uuid), String>,
}
pub fn completion_path(sessions: &Path, tab: Uuid, runtime: Uuid) -> PathBuf {
    sessions.join(format!("{tab}-{runtime}.activity.json"))
}
impl ActivityReader {
    /// Completion files are unique to a live process, never restored history.
    pub fn completions(&mut self, runtimes: &[(Uuid, Uuid)], root: &Path) -> Vec<Uuid> {
        self.completions.retain(|key, _| runtimes.contains(key));
        let mut done = Vec::new();
        for (tab, runtime) in runtimes {
            let path = completion_path(&root.join("sessions"), *tab, *runtime);
            if !std::fs::metadata(&path).is_ok_and(|meta| meta.len() <= 4096) {
                continue;
            }
            let Some(id) = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|value| value["event-id"].as_str().map(str::to_owned))
            else {
                continue;
            };
            if self.completions.get(&(*tab, *runtime)) != Some(&id) {
                self.completions.insert((*tab, *runtime), id);
                done.push(*tab);
            }
        }
        done
    }
    /// Background-only. Every tab has an independent bounded incremental cursor.
    pub fn refresh(&mut self, tabs: &[Tab], root: &Path) -> Vec<Update> {
        self.cached
            .retain(|id, _| tabs.iter().any(|tab| tab.id == *id));
        let mut updates = vec![];
        for tab in tabs {
            let key = ConversationKey::for_tab(tab);
            let state = self.cached.entry(tab.id).or_insert_with(|| LogState {
                key: key.clone(),
                source: None,
                cursor: JsonlCursor::default(),
                phase: Phase::Idle,
                seeding: true,
            });
            if state.key != key {
                *state = LogState {
                    key: key.clone(),
                    source: None,
                    cursor: JsonlCursor::default(),
                    phase: Phase::Idle,
                    seeding: true,
                };
            }
            if state.source.as_ref().is_none_or(|path| !path.is_file()) {
                let source = transcript::locate(tab, root);
                if source != state.source {
                    state.source = source;
                    state.cursor = JsonlCursor::default();
                    state.seeding = true;
                    state.phase = Phase::Idle;
                }
            }
            let Some(path) = &state.source else { continue };
            let (reset, records) = state.cursor.read(path);
            if reset {
                state.seeding = true;
                state.phase = Phase::Idle;
            }
            let seeded = state.seeding;
            let mut changed = reset;
            let mut worked = false;
            for record in records {
                if let Some(phase) = phase_from_record(&tab.profile.kind, &record) {
                    changed |= phase != state.phase;
                    worked |= matches!(phase, Phase::Working | Phase::Blocked);
                    state.phase = phase;
                }
            }
            if state.cursor.caught_up(path) {
                state.seeding = false;
            }
            if changed {
                updates.push(Update {
                    key: state.key.clone(),
                    phase: state.phase,
                    seed: seeded,
                    worked,
                });
            }
        }
        updates
    }
}
fn phase_from_record(kind: &CliKind, value: &Value) -> Option<Phase> {
    if *kind == CliKind::Codex && value["type"] == "event_msg" {
        return match value["payload"]["type"].as_str()? {
            "task_started" => Some(Phase::Working),
            "task_complete" | "turn_complete" => Some(Phase::Done),
            "turn_aborted" => Some(Phase::Idle),
            "error" => Some(Phase::Error),
            _ => None,
        };
    }
    if *kind == CliKind::Claude && value["type"] == "system" && value["subtype"] == "turn_duration"
    {
        return Some(Phase::Done);
    }
    let payload = value
        .get("message")
        .or_else(|| value.get("payload"))
        .unwrap_or(value);
    let stop = payload["stopReason"]
        .as_str()
        .or_else(|| payload["stop_reason"].as_str());
    if payload["role"] == "assistant" {
        match stop {
            Some("toolUse" | "tool_use") => return Some(Phase::Working),
            Some("error") => return Some(Phase::Error),
            Some("aborted") => return Some(Phase::Idle),
            Some("stop" | "end_turn" | "max_tokens" | "length" | "stop_sequence") => {
                return Some(Phase::Done);
            }
            _ => {}
        }
    }
    let message = transcript::message_from(value)?;
    if message.role == Role::User {
        if crate::conversation::is_context(&message.text) {
            return None;
        }
        return Some(Phase::Working);
    }
    if message.role == Role::Tool {
        return Some(Phase::Working);
    }
    if message.role != Role::Assistant {
        return None;
    }
    if *kind == CliKind::Pi {
        Some(Phase::Done)
    } else {
        Some(Phase::Working)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vt_parser_delivers_fragmented_program_status_reports() {
        let mut buffer = crate::terminal::Buffer::new(10, 80, std::path::PathBuf::from("."));
        for byte in b"\x1b]7501;state=working:app=fixture\x1b\\" {
            buffer.process(&[*byte]);
        }
        assert_eq!(
            buffer.parser.callbacks().activity.indicator(true),
            Indicator::Working
        );
        buffer.process(b"\x1b]7501;state=done\x07");
        assert_eq!(
            buffer.parser.callbacks().activity.indicator(true),
            Indicator::Completed
        );
        let mut restored = crate::terminal::Buffer::new(10, 80, std::path::PathBuf::from("."));
        restored.replay(b"\x1b]7501;state=working\x1b\\\x1b]7501;state=done\x07");
        assert_eq!(
            restored.parser.callbacks().activity.indicator(true),
            Indicator::None
        );
    }
    #[test]
    fn managed_completion_is_bound_to_runtime_and_survives_log_seeding() {
        let root = std::env::temp_dir().join(format!("vibe-activity-notify-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        let tab = Uuid::new_v4();
        let runtime = Uuid::new_v4();
        let path = completion_path(&root.join("sessions"), tab, runtime);
        std::fs::write(&path, r#"{"event-id":"first"}"#).unwrap();
        let mut reader = ActivityReader::default();
        assert_eq!(reader.completions(&[(tab, runtime)], &root), vec![tab]);
        assert!(reader.completions(&[(tab, runtime)], &root).is_empty());
        assert!(
            reader
                .completions(&[(tab, Uuid::new_v4())], &root)
                .is_empty()
        );
        let mut activity = Activity::default();
        activity.submitted();
        assert_eq!(activity.indicator(true), Indicator::Working);
        activity.managed_completed();
        assert_eq!(activity.indicator(true), Indicator::Completed);
        assert!(!activity.fallback(Phase::Done, true));
        assert_eq!(activity.indicator(true), Indicator::Completed);
        activity.acknowledge();
        activity.submitted();
        activity.interrupted();
        activity.managed_completed();
        assert_eq!(activity.indicator(true), Indicator::None);
        activity.submitted();
        activity.managed_completed();
        assert_eq!(activity.indicator(true), Indicator::Completed);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn reports_acknowledgement_and_new_turns() {
        let mut activity = Activity::default();
        activity.osc("state=idle:app=codex");
        assert_eq!(activity.indicator(true), Indicator::None);
        activity.osc("state=working:app=codex");
        assert_eq!(activity.indicator(true), Indicator::Working);
        activity.acknowledge();
        assert_eq!(activity.indicator(true), Indicator::Working);
        activity.osc("state=done:app=codex");
        assert_eq!(activity.indicator(true), Indicator::Completed);
        activity.acknowledge();
        activity.osc("state=done:app=codex");
        assert_eq!(activity.indicator(true), Indicator::None);
        activity.osc("state=working");
        activity.osc("state=blocked:kind=permission");
        assert_eq!(activity.indicator(true), Indicator::Blocked);
        activity.osc("state=error");
        assert_eq!(activity.indicator(true), Indicator::Failed);
        activity.acknowledge();
        assert_eq!(activity.indicator(true), Indicator::None);
    }
    #[test]
    fn children_hold_parent_completion_and_idle_never_fakes_a_success() {
        let mut activity = Activity::default();
        activity.osc("state=done");
        assert_eq!(activity.indicator(true), Indicator::None);
        activity.osc("state=working");
        activity.osc("id=child:state=working");
        activity.osc("state=done");
        assert_eq!(activity.indicator(true), Indicator::Working);
        activity.osc("id=child:state=done");
        assert_eq!(activity.indicator(false), Indicator::Completed);
        activity.acknowledge();
        activity.osc("state=working");
        activity.osc("state=idle");
        assert_eq!(activity.indicator(true), Indicator::None);
        assert!(!activity.osc("id=bad//child:state=done"));
        assert!(!activity.osc(&format!("state=working:{}", "x".repeat(5000))));
    }
    #[test]
    fn native_wins_over_transcript_and_replay_is_seeded() {
        let mut activity = Activity::default();
        activity.fallback(Phase::Working, true);
        activity.fallback(Phase::Done, true);
        assert_eq!(activity.indicator(true), Indicator::None);
        activity.fallback(Phase::Working, false);
        activity.fallback(Phase::Done, false);
        assert_eq!(activity.indicator(true), Indicator::Completed);
        activity.osc("state=working");
        assert!(!activity.fallback(Phase::Done, false));
        assert_eq!(activity.indicator(true), Indicator::Working);
    }
    #[test]
    fn old_pi_logs_only_complete_after_a_new_turn() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("vibe-activity-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("sessions")).unwrap();
        let mut profile = crate::model::CliProfile::default_shell();
        profile.kind = CliKind::Pi;
        let tab = Tab::new(profile, root.clone());
        let path = root.join("sessions").join(format!("{}.jsonl", tab.id));
        fs::write(&path,"{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"old\"}}\n{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"content\":\"old answer\",\"stopReason\":\"stop\"}}\n").unwrap();
        let mut reader = ActivityReader::default();
        let initial = reader.refresh(&[tab.clone()], &root);
        assert!(initial[0].seed);
        assert_eq!(initial[0].phase, Phase::Done);
        use std::io::Write;
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            file,
            "{{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":\"new\"}}}}"
        )
        .unwrap();
        let working = reader.refresh(&[tab.clone()], &root);
        assert!(!working[0].seed);
        assert_eq!(working[0].phase, Phase::Working);
        writeln!(file,"{{\"type\":\"message\",\"message\":{{\"role\":\"assistant\",\"content\":\"done\",\"stopReason\":\"stop\"}}}}").unwrap();
        assert_eq!(reader.refresh(&[tab], &root)[0].phase, Phase::Done);
        fs::remove_dir_all(root).unwrap();
    }
}
