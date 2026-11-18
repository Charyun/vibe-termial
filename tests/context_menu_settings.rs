use vibe_terminal::{model::AppData, store::Store};

#[test]
fn right_click_menu_defaults_off_and_persists_both_states() {
    let root = std::env::temp_dir().join(format!("vibe-menu-setting-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = Store { root: root.clone() };
    assert!(!store.load().unwrap().right_click_menu);
    // Existing installations have no right_click_menu field.
    let mut legacy = serde_json::to_value(AppData::default()).unwrap();
    legacy.as_object_mut().unwrap().remove("right_click_menu");
    std::fs::write(root.join("state.json"), serde_json::to_vec(&legacy).unwrap()).unwrap();
    let mut data = store.load().unwrap();
    assert!(!data.right_click_menu);
    for enabled in [true, false] {
        data.right_click_menu = enabled;
        store.save(&data).unwrap();
        assert_eq!(store.load().unwrap().right_click_menu, enabled);
    }
    std::fs::remove_dir_all(root).unwrap();
}
