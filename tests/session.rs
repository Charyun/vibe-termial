use std::{fs, path::PathBuf};
use uuid::Uuid;
use vibe_terminal::{
    model::{AppData, CliProfile, Tab, WorkspacePage},
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
fn old_workspace_records_load_as_terminal_pages() {
    let (root, _store, data) = setup();
    let mut value = serde_json::to_value(data).unwrap();
    value["workspaces"][0]
        .as_object_mut()
        .unwrap()
        .remove("page");
    let restored: AppData = serde_json::from_value(value).unwrap();
    assert_eq!(restored.workspaces[0].page, WorkspacePage::Terminal);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn workspace_pages_and_terminal_history_are_loaded_on_demand() {
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
    session.overview();
    assert_eq!(session.page(), WorkspacePage::Overview);
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
