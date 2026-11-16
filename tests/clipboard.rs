//! Exercise image-paste transport through real PTYs; never make model requests.
#![cfg(feature = "desktop")]
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use uuid::Uuid;
use vibe_terminal::{
    clipboard::{image_path_text, save_image_png},
    model::{AppData, CliKind, CliProfile},
    session::Session,
    store::Store,
};
fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(Instant::now() < deadline, "clipboard PTY fixture timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn profile(helper: &std::path::Path, receipt: &std::path::Path) -> CliProfile {
    CliProfile {
        id: "clipboard-fixture".into(),
        name: "Clipboard fixture".into(),
        command: "node".into(),
        args: vec![
            helper.to_string_lossy().into_owned(),
            receipt.to_string_lossy().into_owned(),
        ],
        kind: CliKind::Custom,
    }
}
fn ready(session: &Session, id: Uuid) -> bool {
    session.terminals[&id]
        .buffer
        .lock()
        .unwrap()
        .parser
        .screen()
        .contents()
        .contains("CLIPBOARD_FIXTURE_READY")
}
#[test]
fn native_image_keys_and_staged_image_path_use_original_process_without_submit() {
    let root = std::env::temp_dir().join(format!("vibe clipboard 中文 {}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let helper = root.join("agent.cjs");
    fs::write(&helper, include_str!("fixtures/clipboard-agent.cjs")).unwrap();
    let mut data = AppData::default();
    data.add_workspace(&root).unwrap();
    let mut session = Session::new(Store { root: root.clone() }, data, None);
    let first_receipt = root.join("first.bin");
    session.new_tab(profile(&helper, &first_receipt));
    let first = session.selected().unwrap();
    wait_for(|| ready(&session, first));
    let target = session.capture_paste_target(first).unwrap();
    session.send_paste_shortcut(&target, b"\x16").unwrap();
    wait_for(|| fs::read(&first_receipt).is_ok_and(|bytes| bytes == b"\x16"));
    session.send_paste_shortcut(&target, b"\x1bv").unwrap();
    wait_for(|| fs::read(&first_receipt).is_ok_and(|bytes| bytes == b"\x16\x1bv"));
    let second_receipt = root.join("second.bin");
    session.new_tab(profile(&helper, &second_receipt));
    let second = session.selected().unwrap();
    wait_for(|| ready(&session, second));
    let mut source = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(4, 3)
        .write_to(&mut source, image::ImageFormat::Png)
        .unwrap();
    let png: PathBuf = save_image_png(source.get_ref(), &root).unwrap();
    session.paste_image_path(&target, &png).unwrap();
    let expected = format!("\x16\x1bv\x1b[200~{}\x1b[201~", image_path_text(&png)).into_bytes();
    wait_for(|| fs::read(&first_receipt).is_ok_and(|bytes| bytes == expected));
    assert!(
        fs::read(&second_receipt).unwrap().is_empty(),
        "late paste must not reach the newly selected tab"
    );
    assert!(
        !expected.contains(&b'\r') && !expected.contains(&b'\n'),
        "paste must not submit the prompt"
    );
    assert!(
        png.is_file(),
        "the Agent must still be able to read the staged image later"
    );
    session.restart(first);
    wait_for(|| ready(&session, first));
    assert!(!session.paste_target_current(&target));
    assert!(session.paste_image_path(&target, &png).is_err());
    assert!(session.send_paste_shortcut(&target, b"\x16").is_err());
    session.close_tab(first);
    assert!(session.capture_paste_target(first).is_err());
    drop(session);
    fs::remove_dir_all(root).unwrap();
}
