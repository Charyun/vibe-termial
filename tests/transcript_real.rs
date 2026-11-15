//! 用本机真实 Codex 会话文件验证解析（需要本机存在 ~/.codex/sessions）。
use vibe_terminal::transcript;

#[test]
#[ignore = "requires local Codex session files under ~/.codex/sessions"]
fn parses_a_real_codex_rollout() {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .expect("home");
    let root = std::path::PathBuf::from(home)
        .join(".codex")
        .join("sessions");
    let newest = newest_jsonl(&root).expect("no codex session file found");
    let parsed = transcript::read(&newest).expect("read transcript");
    println!(
        "parsed {} messages from {}",
        parsed.messages.len(),
        parsed.source.display()
    );
    assert!(!parsed.messages.is_empty(), "transcript was empty");
    assert!(
        parsed
            .messages
            .iter()
            .any(|m| m.role == transcript::Role::Assistant)
    );
}

fn newest_jsonl(directory: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(directory).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        let candidate = if path.is_dir() {
            newest_jsonl(&path)
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            Some(path)
        } else {
            None
        };
        if let Some(candidate) = candidate {
            let stamp = std::fs::metadata(&candidate)
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().is_none_or(|(current, _)| stamp > *current) {
                best = Some((stamp, candidate));
            }
        }
    }
    best.map(|(_, path)| path)
}
