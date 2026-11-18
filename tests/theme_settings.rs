use std::fs;
use uuid::Uuid;
use vibe_terminal::{
    model::{AppData, WindowMaterial},
    store::Store,
};

#[test]
fn window_material_defaults_to_mica_and_preserves_each_selection() {
    let root = std::env::temp_dir().join(format!("vibe-theme-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store = Store { root: root.clone() };
    let mut legacy = serde_json::to_value(AppData::default()).unwrap();
    legacy.as_object_mut().unwrap().remove("window_material");
    let mut data: AppData = serde_json::from_value(legacy).unwrap();
    assert_eq!(data.window_material, WindowMaterial::Mica);
    for material in [
        WindowMaterial::Mica,
        WindowMaterial::MicaAlt,
        WindowMaterial::Acrylic,
    ] {
        data.window_material = material;
        store.save(&data).unwrap();
        assert_eq!(store.load().unwrap().window_material, material);
    }
    fs::remove_dir_all(root).unwrap();
}
