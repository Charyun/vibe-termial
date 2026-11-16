//! Incremental background reads of titles belonging to specific Agent sessions.
use crate::{
    model::{CliKind, Tab},
    transcript::{self, Role},
};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use uuid::Uuid;

use crate::jsonl::JsonlCursor;

#[derive(Default)]
struct CachedSession {
    session_id: Option<String>,
    generation: u64,
    source: Option<PathBuf>,
    cursor: JsonlCursor,
    first_prompt: Option<String>,
    native_title: Option<String>,
}

pub struct SummaryUpdate {
    pub tab_id: Uuid,
    pub session_id: Option<String>,
    pub generation: u64,
    pub title: String,
}
impl SummaryUpdate {
    /// Ignore results from a conversation that was restarted while reading.
    pub fn apply(&self, tab: &mut Tab) -> bool {
        if tab.id != self.tab_id
            || tab.session_id != self.session_id
            || tab.conversation_generation != self.generation
            || !tab.accepts_session_summary()
            || tab.session_summary.as_ref() == Some(&self.title)
        {
            return false;
        }
        tab.session_summary = Some(self.title.clone());
        true
    }
}

#[derive(Default)]
pub struct TitleTracker {
    sessions: HashMap<Uuid, CachedSession>,
    index: JsonlCursor,
    codex_titles: HashMap<String, String>,
}
impl TitleTracker {
    /// Run on a worker thread, never while borrowing the native window.
    pub fn refresh(&mut self, tabs: &[Tab], data_root: &Path) -> Vec<SummaryUpdate> {
        let index = transcript::agent_home("CODEX_HOME", ".codex")
            .map(|root| root.join("session_index.jsonl"));
        self.refresh_from(tabs, data_root, index.as_deref())
    }

    fn refresh_from(
        &mut self,
        tabs: &[Tab],
        data_root: &Path,
        index: Option<&Path>,
    ) -> Vec<SummaryUpdate> {
        if tabs.iter().any(|tab| tab.profile.kind == CliKind::Codex)
            && let Some(path) = index
        {
            let (reset, records) = self.index.read(path);
            if reset {
                self.codex_titles.clear()
            }
            for value in records {
                if let (Some(id), Some(name)) =
                    (value["id"].as_str(), value["thread_name"].as_str())
                    && let Some(name) = short_title(name)
                {
                    self.codex_titles.insert(id.to_owned(), name);
                }
            }
        }
        self.sessions
            .retain(|id, _| tabs.iter().any(|tab| tab.id == *id));
        let mut updates = Vec::new();
        for tab in tabs.iter().filter(|tab| tab.accepts_session_summary()) {
            let cached = self.sessions.entry(tab.id).or_default();
            if cached.session_id != tab.session_id
                || cached.generation != tab.conversation_generation
            {
                *cached = CachedSession {
                    session_id: tab.session_id.clone(),
                    generation: tab.conversation_generation,
                    ..Default::default()
                };
            }
            if cached.source.as_ref().is_none_or(|path| !path.is_file()) {
                cached.source = transcript::locate(tab, data_root);
            }
            if let Some(path) = &cached.source {
                let (reset, records) = cached.cursor.read(path);
                if reset {
                    cached.first_prompt = None;
                    cached.native_title = None;
                }
                for value in records {
                    if let Some(title) = native_title(&value) {
                        cached.native_title = Some(title);
                    }
                    if cached.first_prompt.is_none()
                        && let Some(message) = transcript::message_from(&value)
                            .or_else(|| value.get("message").and_then(transcript::message_from))
                        && message.role == Role::User
                        && !is_context(&message.text)
                    {
                        cached.first_prompt = short_title(&message.text);
                    }
                }
            }
            let indexed = (tab.profile.kind == CliKind::Codex)
                .then(|| {
                    tab.session_id
                        .as_ref()
                        .and_then(|id| self.codex_titles.get(id))
                })
                .flatten();
            if let Some(title) = indexed
                .or(cached.native_title.as_ref())
                .or(cached.first_prompt.as_ref())
            {
                updates.push(SummaryUpdate {
                    tab_id: tab.id,
                    session_id: tab.session_id.clone(),
                    generation: tab.conversation_generation,
                    title: title.clone(),
                });
            }
        }
        updates
    }
}

fn native_title(value: &Value) -> Option<String> {
    let title = match value["type"].as_str()? {
        "session_info" => value["name"].as_str(),
        "custom-title" => value["customTitle"].as_str(),
        "summary" => value["summary"].as_str(),
        _ => None,
    }?;
    short_title(title)
}
fn is_context(text: &str) -> bool {
    let text = text.trim_start();
    [
        "# AGENTS.md instructions",
        "<environment_context>",
        "<permissions instructions>",
        "<turn_aborted>",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}
fn short_title(text: &str) -> Option<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized: String = normalized.chars().filter(|ch| !ch.is_control()).collect();
    if normalized.is_empty() {
        return None;
    }
    let mut title: String = normalized.chars().take(40).collect();
    if normalized.chars().count() > 40 {
        title.push('…');
    }
    Some(title)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write};
    fn fixture() -> (PathBuf, Tab) {
        let root = std::env::temp_dir().join(format!("vibe-title-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("sessions")).unwrap();
        let mut profile = crate::model::CliProfile::default_shell();
        profile.name = "Pi".into();
        profile.kind = CliKind::Pi;
        let tab = Tab::new(profile, root.clone());
        (root, tab)
    }
    #[test]
    fn pi_prompt_then_native_title_and_partial_append() {
        let (root, mut tab) = fixture();
        let path = root.join("sessions").join(format!("{}.jsonl", tab.id));
        fs::write(&path, "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"修复输入法位置\\n支持中文\"}}\n").unwrap();
        let mut tracker = TitleTracker::default();
        let updates = tracker.refresh_from(&[tab.clone()], &root, None);
        assert!(updates[0].apply(&mut tab));
        assert_eq!(tab.display_title(), "修复输入法位置 支持中文");
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(file, "{{\"type\":\"session_info\",\"name\":\"输入法定位").unwrap();
        assert_eq!(
            tracker.refresh_from(&[tab.clone()], &root, None)[0].title,
            "修复输入法位置 支持中文"
        );
        writeln!(file, "修复\"}}").unwrap();
        let updates = tracker.refresh_from(&[tab.clone()], &root, None);
        assert_eq!(updates[0].title, "输入法定位修复");
        tab.rename("我的固定标签".into());
        assert!(!updates[0].apply(&mut tab));
        assert_eq!(tab.display_title(), "我的固定标签");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn codex_index_is_bound_to_exact_session_id() {
        let (root, mut tab) = fixture();
        tab.profile.kind = CliKind::Codex;
        tab.session_id = Some(Uuid::new_v4().to_string());
        let index = root.join("index.jsonl");
        let record = serde_json::json!({"id":tab.session_id,"thread_name":"修复终端输入法"});
        fs::write(
            &index,
            format!(
                "{}\n{}\n",
                record,
                serde_json::json!({"id":"different-session", "thread_name":"别的会话"})
            ),
        )
        .unwrap();
        let mut tracker = TitleTracker::default();
        let updates = tracker.refresh_from(&[tab.clone()], &root, Some(&index));
        assert_eq!(updates[0].title, "修复终端输入法");
        tab.conversation_generation += 1;
        assert!(!updates[0].apply(&mut tab));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn legacy_custom_names_survive_and_unicode_titles_are_bounded() {
        let (root, mut tab) = fixture();
        tab.session_summary = Some("自动介绍".into());
        tab.title = "自己起的名字".into();
        assert_eq!(tab.display_title(), "自己起的名字");
        tab.title = "Pi 2".into();
        assert_eq!(tab.display_title(), "自动介绍");
        tab.rename("Pi 3".into());
        assert_eq!(tab.display_title(), "Pi 3");
        assert_eq!(short_title(&"中".repeat(50)).unwrap().chars().count(), 41);
        assert!(is_context("# AGENTS.md instructions for project"));
        fs::remove_dir_all(root).unwrap();
    }
}
