#![cfg(feature = "desktop")]
//! The real desktop binary's completion helper must stay headless and scope status to its runtime.
use std::fs;
use uuid::Uuid;
use vibe_terminal::activity::{ActivityReader, completion_path};
#[test]
fn codex_completion_helper_records_session_and_current_runtime_without_a_window() {
    let root = std::env::temp_dir().join(format!("vibe-codex-activity-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
    let tab = Uuid::new_v4();
    let runtime = Uuid::new_v4();
    let thread = Uuid::new_v4();
    let marker = root.join("sessions").join(format!("{tab}.codex.json"));
    let completion = completion_path(&root.join("sessions"), tab, runtime);
    let payload=serde_json::json!({"type":"agent-turn-complete","thread-id":thread,"last-assistant-message":"x".repeat(17_000)}).to_string();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_vibe-terminal"))
        .arg("--record-codex-session")
        .arg(&marker)
        .arg(&completion)
        .arg(payload)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&marker).unwrap()).unwrap()["thread-id"],
        thread.to_string()
    );
    let mut reader = ActivityReader::default();
    assert_eq!(reader.completions(&[(tab, runtime)], &root), vec![tab]);
    assert!(reader.completions(&[(tab, runtime)], &root).is_empty());
    assert!(
        reader
            .completions(&[(tab, Uuid::new_v4())], &root)
            .is_empty()
    );
    fs::remove_dir_all(root).unwrap();
}
