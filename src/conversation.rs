//! One Agent session, shared by terminal and conversation presentations.
use crate::{
    jsonl::JsonlCursor,
    model::{CliKind, Tab},
    session::Session,
    terminal::Buffer,
    transcript::{self, Message, Role},
};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationKey {
    pub tab_id: Uuid,
    pub session_id: Option<String>,
    pub generation: u64,
}
impl ConversationKey {
    pub fn for_tab(tab: &Tab) -> Self {
        Self {
            tab_id: tab.id,
            session_id: tab.session_id.clone(),
            generation: tab.conversation_generation,
        }
    }
    pub fn matches(&self, tab: &Tab) -> bool {
        *self == Self::for_tab(tab)
    }
}

#[derive(Clone)]
pub struct ConversationSnapshot {
    pub key: ConversationKey,
    pub source: Option<PathBuf>,
    pub messages: Arc<Vec<Message>>,
}

struct CachedConversation {
    key: ConversationKey,
    source: Option<PathBuf>,
    cursor: JsonlCursor,
    messages: Vec<Message>,
    slots: HashMap<String, usize>,
}
#[derive(Default)]
pub struct ConversationReader {
    cached: Option<CachedConversation>,
}
impl ConversationReader {
    /// Call on a background worker. A read has a bounded budget and resumes on the next poll.
    pub fn refresh(&mut self, tab: &Tab, root: &Path) -> Option<ConversationSnapshot> {
        let key = ConversationKey::for_tab(tab);
        let new_session = self.cached.as_ref().is_none_or(|cached| cached.key != key);
        if new_session {
            self.cached = Some(CachedConversation {
                key,
                source: None,
                cursor: JsonlCursor::default(),
                messages: vec![],
                slots: HashMap::new(),
            });
        }
        let cached = self.cached.as_mut().unwrap();
        let mut changed = new_session;
        if cached.source.as_ref().is_none_or(|path| !path.is_file()) {
            let source = transcript::locate(tab, root);
            changed |= cached.source != source;
            cached.source = source;
        }
        if let Some(path) = &cached.source {
            let (reset, records) = cached.cursor.read(path);
            if reset {
                cached.messages.clear();
                cached.slots.clear();
                changed = true;
            }
            for value in records {
                let Some(message) = transcript::message_from(&value) else {
                    continue;
                };
                if message.role == Role::User && is_context(&message.text) {
                    continue;
                }
                let id = value
                    .get("message")
                    .and_then(|message| message.get("id"))
                    .or_else(|| value.get("id"))
                    .and_then(|id| id.as_str());
                if let Some(index) = id.and_then(|id| cached.slots.get(id)).copied() {
                    if cached.messages[index] != message {
                        cached.messages[index] = message;
                        changed = true;
                    }
                } else {
                    if let Some(id) = id {
                        cached.slots.insert(id.to_owned(), cached.messages.len());
                    }
                    cached.messages.push(message);
                    changed = true;
                }
            }
        }
        changed.then(|| ConversationSnapshot {
            key: cached.key.clone(),
            source: cached.source.clone(),
            messages: Arc::new(cached.messages.clone()),
        })
    }
}
pub(crate) fn is_context(text: &str) -> bool {
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

#[derive(Clone)]
pub struct PendingMessage {
    pub text: String,
    pub after: usize,
}
impl PendingMessage {
    pub fn recorded(&self, messages: &[Message]) -> bool {
        messages
            .get(self.after..)
            .unwrap_or_default()
            .iter()
            .any(|message| message.role == Role::User && message.text.trim() == self.text.trim())
    }
}

/// A delayed Enter is tied to the original process, not whichever tab is selected later.
pub struct MessageSubmission {
    key: ConversationKey,
    turn: bool,
    buffer: Arc<Mutex<Buffer>>,
}
pub fn message_paste_bytes(text: &str) -> Result<Vec<u8>> {
    if text.trim().is_empty() {
        bail!("请输入消息")
    }
    if text.len() > 256 * 1024 {
        bail!("消息超过 256 KB")
    }
    if text
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\r' | '\n' | '\t'))
    {
        bail!("消息包含无法发送的控制字符")
    }
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    Ok(format!("\x1b[200~{text}\x1b[201~").into_bytes())
}
/// Recognize Pi's single-line editor between its horizontal borders. Unknown
/// layouts are left alone; multiline drafts must be handled in the terminal.
fn pi_editor_lines(screen: &vt100::Screen) -> Option<Vec<String>> {
    let (rows, cols) = screen.size();
    let caret = screen.cursor_position().0 as usize;
    let lines: Vec<_> = screen.rows(0, cols).collect();
    let border = |line: &str| {
        let text = line.trim();
        text.chars().count() >= cols as usize / 2
            && text.chars().all(|ch| matches!(ch, '─' | '━' | ' '))
    };
    let top = (0..caret).rev().find(|row| border(&lines[*row]))?;
    let bottom = (caret + 1..rows as usize).find(|row| border(&lines[*row]))?;
    Some(
        lines[top + 1..bottom]
            .iter()
            .map(|line| line.trim_end().to_owned())
            .collect(),
    )
}

impl Session {
    pub fn content_draft(&self, id: Uuid) -> Option<String> {
        if self.data.tab(id)?.profile.kind != CliKind::Pi {
            return None;
        }
        let buffer = self.terminals.get(&id)?.buffer.lock().ok()?;
        let lines = pi_editor_lines(buffer.parser.screen())?;
        (lines.len() == 1).then(|| lines[0].clone())
    }
    pub fn begin_message(&self, id: Uuid, text: &str) -> Result<MessageSubmission> {
        self.begin_message_with_image(id, text, false)
    }
    /// An explicitly pasted native image also makes an otherwise empty message sendable.
    pub fn begin_message_with_image(
        &self,
        id: Uuid,
        text: &str,
        has_image: bool,
    ) -> Result<MessageSubmission> {
        if self.data.is_remote_tab(id) {
            bail!("SSH 远程会话请在终端中交互，暂未同步远端会话记录");
        }
        let tab = self.data.tab(id).context("会话标签已关闭")?;
        if !matches!(
            tab.profile.kind,
            CliKind::Pi | CliKind::Codex | CliKind::Claude
        ) {
            bail!("当前标签不是 Agent 会话")
        }
        let terminal = self.terminals.get(&id).context("请先恢复会话")?;
        let mut bytes = if has_image && text.trim().is_empty() {
            Vec::new()
        } else {
            message_paste_bytes(text)?
        };
        if tab.profile.kind == CliKind::Pi {
            let buffer = terminal
                .buffer
                .lock()
                .map_err(|_| anyhow::anyhow!("终端暂时不可用"))?;
            if let Some(lines) = pi_editor_lines(buffer.parser.screen())
                && lines.iter().any(|line| !line.is_empty())
            {
                if lines.len() != 1 {
                    bail!("终端中已有多行草稿，请先在终端提交或清空后再发送")
                }
                // Replace the editor's existing draft instead of concatenating
                // the composer text onto it. Ctrl+A then Ctrl+K clears one line.
                bytes.splice(0..0, [1, 11]);
            }
        }
        if !bytes.is_empty() {
            terminal.send(bytes)?;
        }
        Ok(MessageSubmission {
            key: ConversationKey::for_tab(tab),
            turn: has_image || !text.trim_start().starts_with('/'),
            buffer: terminal.buffer.clone(),
        })
    }
    pub fn finish_message(&self, submission: MessageSubmission) -> Result<()> {
        let tab = self
            .data
            .tab(submission.key.tab_id)
            .context("会话标签已关闭，消息未提交")?;
        let terminal = self
            .terminals
            .get(&tab.id)
            .context("会话已停止，消息未提交")?;
        if submission.key.generation != tab.conversation_generation
            || submission
                .key
                .session_id
                .as_ref()
                .is_some_and(|id| tab.session_id.as_ref() != Some(id))
            || !Arc::ptr_eq(&submission.buffer, &terminal.buffer)
        {
            bail!("会话已经更换，消息未提交；请检查终端后重试")
        }
        let result = terminal.send(vec![b'\r']);
        if result.is_ok() && submission.turn {
            self.note_codex_submission(tab.id);
        }
        result
    }
    pub fn interrupt_message(&self, id: Uuid) -> Result<()> {
        let terminal = self.terminals.get(&id).context("会话未运行")?;
        let result = terminal.send(vec![27]);
        if result.is_ok()
            && !self.data.is_remote_tab(id)
            && self
                .data
                .tab(id)
                .is_some_and(|tab| tab.profile.kind == CliKind::Codex)
            && let Ok(mut buffer) = terminal.buffer.lock()
        {
            buffer.parser.callbacks_mut().activity.interrupted();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write};
    #[test]
    fn paste_is_framed_and_never_contains_submit_or_control_sequences() {
        assert_eq!(
            message_paste_bytes("你好\r\nworld").unwrap(),
            "\x1b[200~你好\nworld\x1b[201~".as_bytes()
        );
        assert!(message_paste_bytes("\x1b[201~\r/clear").is_err());
        assert!(message_paste_bytes("  \n").is_err());
    }
    #[test]
    fn pending_echo_only_matches_new_messages() {
        let pending = PendingMessage {
            text: "same prompt".into(),
            after: 1,
        };
        let message = Message {
            role: Role::User,
            text: "same prompt".into(),
        };
        assert!(!pending.recorded(&[message.clone()]));
        assert!(pending.recorded(&[message.clone(), message]));
    }
    #[test]
    fn conversation_follows_partial_appends_and_replaces_streaming_messages() {
        let root = std::env::temp_dir().join(format!("vibe-conversation-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("sessions")).unwrap();
        let mut profile = crate::model::CliProfile::default_shell();
        profile.kind = CliKind::Pi;
        let mut tab = Tab::new(profile, root.clone());
        let path = root.join("sessions").join(format!("{}.jsonl", tab.id));
        let mut reader = ConversationReader::default();
        assert!(reader.refresh(&tab, &root).unwrap().messages.is_empty());
        fs::write(&path, "{\"id\":\"u\",\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"你好\"}}\n").unwrap();
        assert_eq!(reader.refresh(&tab, &root).unwrap().messages.len(), 1);
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(file, "{{\"id\":\"a\",\"type\":\"message\",\"message\":{{\"role\":\"assistant\",\"content\":\"正在").unwrap();
        assert!(reader.refresh(&tab, &root).is_none());
        writeln!(file, "回答\"}}}}").unwrap();
        assert_eq!(
            reader.refresh(&tab, &root).unwrap().messages[1].text,
            "正在回答"
        );
        writeln!(file, "{{\"id\":\"a\",\"type\":\"message\",\"message\":{{\"role\":\"assistant\",\"content\":\"回答完成\"}}}}").unwrap();
        let snapshot = reader.refresh(&tab, &root).unwrap();
        assert_eq!(snapshot.messages.len(), 2);
        assert_eq!(snapshot.messages[1].text, "回答完成");
        tab.conversation_generation += 1;
        assert!(!snapshot.key.matches(&tab));
        fs::remove_dir_all(root).unwrap();
    }
}
