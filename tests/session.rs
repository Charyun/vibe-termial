use std::{fs, path::PathBuf};
use uuid::Uuid;
use vibe_terminal::{
    model::{AppData, CliProfile, InputCursorStyle, Tab},
    session::Session,
    store::Store,
};

fn setup() -> (PathBuf, Store, AppData) {
    let root = std::env::temp_dir().join(format!("vibe-session-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("history")).unwrap();
    fs::create_dir_all(root.join("sessions")).unwrap();
    let store = Store { root: root.clone() };
    let mut data = AppData::default();
    data.add_workspace(&root).unwrap();
    (root, store, data)
}
#[test]
fn general_cursor_style_defaults_to_block_and_persists_both_choices() {
    let (root, store, data) = setup();
    let mut legacy = serde_json::to_value(data).unwrap();
    legacy.as_object_mut().unwrap().remove("input_cursor_style");
    let mut restored: AppData = serde_json::from_value(legacy).unwrap();
    assert_eq!(restored.input_cursor_style, InputCursorStyle::Block);
    for style in [InputCursorStyle::Bar, InputCursorStyle::Block] {
        restored.input_cursor_style = style;
        store.save(&restored).unwrap();
        assert_eq!(store.load().unwrap().input_cursor_style, style);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn right_panel_visibility_is_saved_per_workspace_and_defaults_closed_for_legacy_data() {
    let (root, store, mut data) = setup();
    let first = data.selected_workspace.unwrap();
    let second_path = root.join("second");
    fs::create_dir_all(&second_path).unwrap();
    let second = data.add_workspace(&second_path).unwrap();
    assert!(!data.workspace().unwrap().right_panel_open);
    data.selected_workspace = Some(first);
    data.workspace_mut().unwrap().right_panel_open = true;
    store.save(&data).unwrap();
    let mut restored = store.load().unwrap();
    assert!(restored.workspace().unwrap().right_panel_open);
    restored.selected_workspace = Some(second);
    assert!(!restored.workspace().unwrap().right_panel_open);
    let mut legacy = serde_json::to_value(restored).unwrap();
    for workspace in legacy["workspaces"].as_array_mut().unwrap() {
        workspace.as_object_mut().unwrap().remove("right_panel_open");
    }
    let legacy: AppData = serde_json::from_value(legacy).unwrap();
    assert!(legacy.workspaces.iter().all(|workspace| !workspace.right_panel_open));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_page_fields_are_ignored_without_losing_workspace_selection() {
    let (root, _store, data) = setup();
    for page in [None, Some("overview"), Some("terminal")] {
        let mut value = serde_json::to_value(&data).unwrap();
        if let Some(page) = page {
            value["workspaces"][0]["page"] = page.into();
        }
        let restored: AppData = serde_json::from_value(value).unwrap();
        assert_eq!(restored.selected_workspace, data.selected_workspace);
        assert_eq!(
            restored.workspaces[0].directory,
            data.workspaces[0].directory
        );
        assert!(
            serde_json::to_value(restored).unwrap()["workspaces"][0]
                .get("page")
                .is_none()
        );
    }
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn terminal_history_is_loaded_on_demand_and_last_tab_can_close() {
    let (root, store, mut data) = setup();
    let p = CliProfile::defaults().remove(0);
    let first = Tab::new(p.clone(), root.clone());
    let second = Tab::new(p, root.clone());
    let (a, b) = (first.id, second.id);
    let w = data.workspace_mut().unwrap();
    w.tabs = vec![first, second];
    w.selected_tab = Some(a);
    store.save_history(a, b"first terminal").unwrap();
    store.save_history(b, b"second terminal").unwrap();
    let mut session = Session::new(store, data, None);
    assert!(session.terminals.is_empty());
    session.select_tab(a);
    assert_eq!(session.terminals.len(), 1);
    assert!(!session.terminals[&a].loaded());
    assert!(
        session.terminals[&a]
            .buffer
            .lock()
            .unwrap()
            .parser
            .screen()
            .contents()
            .contains("first terminal")
    );
    session.select_tab(b);
    assert_eq!(session.terminals.len(), 1);
    assert!(!session.terminals.contains_key(&a));
    session.close_tab(b);
    assert_eq!(session.selected(), Some(a));
    session.close_tab(a);
    assert_eq!(session.selected(), None);
    assert!(session.data.workspace().unwrap().tabs.is_empty());
    assert!(session.terminals.is_empty());
    drop(session);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn repeated_new_tabs_keep_the_profile_name_and_stay_distinct() {
    let (root, store, data) = setup();
    let mut session = Session::new(store, data, None);
    session.read_only = true;
    let profile = CliProfile::default_shell();
    let name = profile.name.clone();
    for _ in 0..3 {
        session.new_tab(profile.clone());
    }
    let tabs = &session.data.workspace().unwrap().tabs;
    // 标签标题不再追加数字编号，靠 id 与图标区分。
    assert!(tabs.iter().all(|tab| tab.title == name));
    let ids: std::collections::HashSet<_> = tabs.iter().map(|t| t.id).collect();
    assert_eq!(ids.len(), 3);
    drop(session);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn removing_workspace_never_deletes_project_files() {
    let (root, store, data) = setup();
    let id = data.selected_workspace.unwrap();
    let source = root.join("important.rs");
    fs::write(&source, "fn main() {}\n").unwrap();
    let mut session = Session::new(store, data, None);
    session.remove_workspace(id);
    assert!(session.data.workspaces.is_empty());
    assert!(source.is_file());
    drop(session);
    fs::remove_dir_all(root).unwrap();
}
