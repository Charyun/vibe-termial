use std::{fs, path::Path};
use vibe_terminal::paths::{FileLink, Preview};
fn load(path: &Path) -> Preview {
    Preview::load(&FileLink {
        start: 0,
        end: 0,
        path: path.into(),
        line: Some(1),
        column: None,
    })
    .unwrap()
}
#[test]
fn editing_preserves_utf8_bom_and_crlf_and_rejects_external_changes() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&root).unwrap();
    let path = root.join("document.txt");
    fs::write(&path, b"\xef\xbb\xbforiginal\r\n").unwrap();
    let mut document = load(&path);
    assert_eq!(document.content, "original\r\n");
    document.save("edited\n").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"\xef\xbb\xbfedited\r\n");
    fs::write(&path, b"changed elsewhere\n").unwrap();
    assert!(document.save("my pending edit\n").is_err());
    assert_eq!(fs::read(&path).unwrap(), b"changed elsewhere\n");
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn editing_preserves_utf16_and_rejects_readonly_files() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&root).unwrap();
    let path = root.join("document.txt");
    let mut bytes = vec![0xff, 0xfe];
    for unit in "中文\r\n".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    fs::write(&path, bytes).unwrap();
    let mut document = load(&path);
    document.save("修改\n").unwrap();
    let bytes = fs::read(&path).unwrap();
    assert_eq!(&bytes[..2], &[0xff, 0xfe]);
    assert_eq!(
        String::from_utf16(
            &bytes[2..]
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>()
        )
        .unwrap(),
        "修改\r\n"
    );
    let original = fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    assert!(document.save("blocked").is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    fs::set_permissions(&path, original).unwrap();
    fs::remove_dir_all(root).unwrap();
}
