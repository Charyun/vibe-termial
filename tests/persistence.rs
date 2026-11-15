use vibe_terminal::{
    model::{AppData, CliKind, CliProfile, Tab},
    store::Store,
};
#[test]
fn workspace_roundtrip_is_atomic_and_duplicate_directories_are_reused() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(root.join("project with spaces")).unwrap();
    let store = Store { root: root.clone() };
    let mut data = AppData::default();
    let id = data
        .add_workspace(&root.join("project with spaces"))
        .unwrap();
    assert_eq!(
        data.add_workspace(&root.join("project with spaces"))
            .unwrap(),
        id
    );
    assert_eq!(data.workspaces.len(), 1);
    store.save(&data).unwrap();
    data.workspaces[0].name = "renamed".into();
    store.save(&data).unwrap();
    assert_eq!(store.load().unwrap().workspaces[0].name, "renamed");
    std::fs::write(root.join("state.json"), "invalid json").unwrap();
    assert!(store.load().is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("state.json")).unwrap(),
        "invalid json"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn legacy_verbatim_directories_are_normalized_on_load() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let store = Store { root: root.clone() };
    let mut data = AppData::default();
    let id = data.add_workspace(&project).unwrap();
    data.selected_workspace = Some(id);
    data.workspaces[0].directory = std::path::PathBuf::from(format!(r"\\?\{}", project.display()));
    let mut tab = Tab::new(
        CliProfile::default_shell(),
        data.workspaces[0].directory.clone(),
    );
    tab.cwd = std::path::PathBuf::from(r"\\?\C:\Windows");
    data.workspaces[0].tabs.push(tab);
    store.save(&data).unwrap();

    let loaded = store.load().unwrap();
    assert_eq!(loaded.workspaces[0].directory, project);
    assert_eq!(
        loaded.workspaces[0].tabs[0].cwd,
        std::path::PathBuf::from(r"C:\Windows")
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn loading_drops_agent_profiles_because_agents_are_detected() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).unwrap();
    let store = Store { root: root.clone() };
    let mut data = AppData::default();
    data.profiles.push(CliProfile {
        id: "pi".into(),
        name: "Pi".into(),
        command: "pi".into(),
        args: vec![],
        kind: CliKind::Pi,
    });
    store.save(&data).unwrap();

    let loaded = store.load().unwrap();
    assert_eq!(loaded.profiles.len(), 2);
    assert!(loaded.profiles.iter().all(|p| p.kind == CliKind::Shell));
    assert!(loaded.profiles.iter().any(|p| p.id == "powershell"));

    std::fs::remove_dir_all(root).unwrap();
}
