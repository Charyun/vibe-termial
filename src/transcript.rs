//! 读取 Agent 自己写下的会话文件，得到可展示的对话内容。
//!
//! 终端输出只适合展示，不适合重建对话；每个 Agent 会把结构化对话写进自己的会话文件，
//! 我们只要知道标签对应的会话身份（Pi 用应用指定的 jsonl，Codex 用 thread-id）就能读到。

use crate::model::{CliKind, Tab};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Tool,
    System,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "你",
            Self::Assistant => "Agent",
            Self::Tool => "工具",
            Self::System => "系统",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

/// 一个标签对应的对话内容。
pub struct Transcript {
    pub source: PathBuf,
    pub messages: Vec<Message>,
}

/// 定位标签对应的会话文件；目前支持 Pi 与 Codex。
pub fn locate(tab: &Tab, data_root: &Path) -> Option<PathBuf> {
    match tab.profile.kind {
        CliKind::Pi => {
            let path = data_root.join("sessions").join(format!("{}.jsonl", tab.id));
            path.is_file().then_some(path)
        }
        CliKind::Codex => tab.session_id.as_deref().and_then(find_codex_session),
        _ => None,
    }
}

/// 在 `~/.codex/sessions` 里按 thread-id 找最新的 rollout 文件。
fn find_codex_session(thread: &str) -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let root = PathBuf::from(home).join(".codex").join("sessions");
    newest_matching(
        &root,
        &|name| name.contains(thread) && name.ends_with(".jsonl"),
        0,
    )
}

fn newest_matching(
    directory: &Path,
    matches: &impl Fn(&str) -> bool,
    depth: usize,
) -> Option<PathBuf> {
    if depth > 4 {
        return None;
    }
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(directory).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = newest_matching(&path, matches, depth + 1) {
                let stamp = std::fs::metadata(&found)
                    .and_then(|meta| meta.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                if best.as_ref().is_none_or(|(current, _)| stamp > *current) {
                    best = Some((stamp, found));
                }
            }
        } else if matches(&entry.file_name().to_string_lossy()) {
            let stamp = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().is_none_or(|(current, _)| stamp > *current) {
                best = Some((stamp, path));
            }
        }
    }
    best.map(|(_, path)| path)
}

pub fn read(path: &Path) -> Result<Transcript> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("无法读取会话文件：{}", path.display()))?;
    Ok(Transcript {
        source: path.to_path_buf(),
        messages: parse(&text),
    })
}

/// 容错解析 JSONL：认识的记录抽成消息，其余（事件、token 统计、未知字段）忽略。
pub fn parse(text: &str) -> Vec<Message> {
    let mut messages = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(line)
            && let Some(message) = message_from(&value)
        {
            messages.push(message);
        }
    }
    messages
}

fn message_from(value: &Value) -> Option<Message> {
    // Codex: response_item -> payload{type:message, role, content[]}
    if value.get("type").and_then(Value::as_str) == Some("response_item") {
        let payload = value.get("payload")?;
        if payload.get("type").and_then(Value::as_str) != Some("message") {
            return None;
        }
        return build_message(payload);
    }
    // Pi / Claude: message -> {role, content[]}
    if value.get("type").and_then(Value::as_str) == Some("message") {
        return build_message(value.get("message")?);
    }
    // 顶层直接是 {role, content}
    if value.get("role").is_some() {
        return build_message(value);
    }
    None
}

fn build_message(payload: &Value) -> Option<Message> {
    let role = match payload.get("role").and_then(Value::as_str)? {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "system" => Role::System,
        "tool" => Role::Tool,
        _ => return None,
    };
    let text = text_of(payload.get("content")?)?;
    (!text.trim().is_empty()).then_some(Message { role, text })
}

fn text_of(content: &Value) -> Option<String> {
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let mut parts: Vec<String> = Vec::new();
            for item in items {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    parts.push(text.to_owned());
                }
            }
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_response_items_and_skips_events() {
        let text = concat!(
            r#"{"timestamp":"t","type":"session_meta","payload":{"session_id":"abc"}}"#,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"请修复登录"}]}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{}}}"#,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"已修复"},{"type":"output_text","text":"并补了测试"}]}}"#,
            "\n",
            "not json at all\n",
        );
        assert_eq!(
            parse(text),
            vec![
                Message {
                    role: Role::User,
                    text: "请修复登录".into()
                },
                Message {
                    role: Role::Assistant,
                    text: "已修复\n并补了测试".into()
                },
            ]
        );
    }

    #[test]
    fn parses_pi_and_plain_message_lines() {
        let text = concat!(
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"你好"}]}}"#,
            "\n",
            r#"{"role":"assistant","content":"在的"}"#,
            "\n",
            r#"{"type":"message","message":{"role":"assistant","content":[]}}"#,
        );
        assert_eq!(
            parse(text),
            vec![
                Message {
                    role: Role::User,
                    text: "你好".into()
                },
                Message {
                    role: Role::Assistant,
                    text: "在的".into()
                },
            ]
        );
    }

    #[test]
    fn locate_pi_uses_the_tab_session_file() {
        let root = std::env::temp_dir().join(format!("vibe-transcript-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        let mut profile = crate::model::CliProfile::defaults().remove(0);
        profile.kind = CliKind::Pi;
        let tab = Tab::new(profile, root.clone());
        let path = root.join("sessions").join(format!("{}.jsonl", tab.id));
        std::fs::write(&path, b"").unwrap();
        assert_eq!(locate(&tab, &root), Some(path));
        std::fs::remove_dir_all(root).unwrap();
    }
}
