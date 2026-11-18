use std::fs;
use uuid::Uuid;
use vibe_terminal::{model::AppData, store::Store};

#[test]
fn pane_widths_restore_defaults_for_legacy_config_and_persist_preferences() {
    let mut legacy = serde_json::to_value(AppData::default()).unwrap();
    legacy.as_object_mut().unwrap().remove("sidebar_width");
    legacy.as_object_mut().unwrap().remove("right_panel_width");
    let mut data: AppData = serde_json::from_value(legacy).unwrap();
    assert_eq!((data.sidebar_width, data.right_panel_width), (224., 440.));
    let root = std::env::temp_dir().join(format!("vibe-width-settings-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store = Store { root: root.clone() };
    data.sidebar_width = 304.;
    data.right_panel_width = 520.;
    store.save(&data).unwrap();
    let restored = store.load().unwrap();
    assert_eq!(
        (restored.sidebar_width, restored.right_panel_width),
        (304., 520.)
    );
    fs::remove_dir_all(root).unwrap();
}
