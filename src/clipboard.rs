//! Clipboard image transport: preserve native Agent shortcuts, otherwise stage a PNG.
use crate::{
    conversation::ConversationKey,
    model::{CliKind, CliProfile},
    session::Session,
    terminal::{Buffer, Terminal},
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImagePaste {
    Native(&'static [u8]),
    FilePath,
}
/// These are image shortcuts, not bracketed text paste. Unknown CLIs get a readable file.
pub fn image_paste(profile: &CliProfile) -> ImagePaste {
    match profile.kind {
        CliKind::Codex => ImagePaste::Native(b"\x16"),
        CliKind::Pi | CliKind::Claude if cfg!(windows) => ImagePaste::Native(b"\x1bv"),
        CliKind::Pi | CliKind::Claude => ImagePaste::Native(b"\x16"),
        CliKind::Custom if matches!(profile.id.as_str(), "gemini" | "opencode") => {
            ImagePaste::Native(b"\x16")
        }
        _ => ImagePaste::FilePath,
    }
}
/// Keep this file after paste: the Agent may read it only when the user submits later.
pub fn save_image_png(bytes: &[u8], directory: &Path) -> Result<PathBuf> {
    let image = image::load_from_memory(bytes).context("剪贴板图片无法解码")?;
    fs::create_dir_all(directory).context("无法创建图片临时目录")?;
    let path = directory.join(format!("clipboard-{}.png", Uuid::new_v4()));
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .context("无法创建临时图片")?;
    let mut writer = BufWriter::new(file);
    let result = (|| {
        image.write_to(&mut writer, image::ImageFormat::Png)?;
        writer.flush()?;
        Ok::<_, anyhow::Error>(())
    })();
    if let Err(error) = result {
        drop(writer);
        let _ = fs::remove_file(&path);
        return Err(error.context("无法保存临时图片"));
    }
    Ok(path)
}
pub fn image_path_text(path: &Path) -> String {
    let path = crate::paths::local_path(path)
        .to_string_lossy()
        .into_owned();
    // Forward slashes also work on Windows and avoid Agent path parsers treating backslashes as escapes.
    let path = if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path
    };
    format!("\"{}\" ", path.replace('"', "\\\""))
}
#[derive(Clone)]
pub struct PasteTarget {
    key: ConversationKey,
    buffer: Arc<Mutex<Buffer>>,
}
impl PasteTarget {
    pub fn tab_id(&self) -> Uuid {
        self.key.tab_id
    }
}
impl Session {
    pub fn capture_paste_target(&self, id: Uuid) -> Result<PasteTarget> {
        let tab = self.data.tab(id).context("会话标签已关闭")?;
        let terminal = self
            .terminals
            .get(&id)
            .filter(|terminal| terminal.running())
            .context("请先恢复会话再粘贴图片")?;
        Ok(PasteTarget {
            key: ConversationKey::for_tab(tab),
            buffer: terminal.buffer.clone(),
        })
    }
    fn paste_terminal(&self, target: &PasteTarget) -> Result<&Terminal> {
        let tab = self.data.tab(target.key.tab_id).context("会话标签已关闭")?;
        let terminal = self
            .terminals
            .get(&tab.id)
            .filter(|terminal| terminal.running())
            .context("会话已停止，请重新粘贴图片")?;
        if target.key.generation != tab.conversation_generation
            || target
                .key
                .session_id
                .as_ref()
                .is_some_and(|id| tab.session_id.as_ref() != Some(id))
            || !Arc::ptr_eq(&target.buffer, &terminal.buffer)
        {
            bail!("会话已更换，请重新粘贴图片")
        }
        Ok(terminal)
    }
    pub fn paste_target_current(&self, target: &PasteTarget) -> bool {
        self.paste_terminal(target).is_ok()
    }
    pub fn send_paste_shortcut(&self, target: &PasteTarget, bytes: &[u8]) -> Result<()> {
        self.paste_terminal(target)?.send(bytes.to_vec())
    }
    pub fn paste_image_path(&self, target: &PasteTarget, path: &Path) -> Result<()> {
        self.paste_terminal(target)?.paste(&image_path_text(path))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_shortcuts_and_unknown_agent_fallback() {
        let mut profile = CliProfile::default_shell();
        profile.kind = CliKind::Codex;
        assert_eq!(image_paste(&profile), ImagePaste::Native(b"\x16"));
        for kind in [CliKind::Pi, CliKind::Claude] {
            profile.kind = kind;
            assert_eq!(
                image_paste(&profile),
                ImagePaste::Native(if cfg!(windows) { b"\x1bv" } else { b"\x16" })
            );
        }
        profile.kind = CliKind::Custom;
        profile.id = "unknown-agent".into();
        assert_eq!(image_paste(&profile), ImagePaste::FilePath);
        for id in ["gemini", "opencode"] {
            profile.id = id.into();
            assert_eq!(image_paste(&profile), ImagePaste::Native(b"\x16"));
        }
    }
    #[test]
    fn clipboard_bitmap_becomes_unique_valid_png_with_quoted_unicode_path() {
        let directory = std::env::temp_dir().join(format!("vibe image 中文 {}", Uuid::new_v4()));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(3, 2)
            .write_to(&mut bytes, image::ImageFormat::Bmp)
            .unwrap();
        let first = save_image_png(bytes.get_ref(), &directory).unwrap();
        let second = save_image_png(bytes.get_ref(), &directory).unwrap();
        assert_ne!(first, second);
        assert!(fs::read(&first).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(
            (
                image::open(&first).unwrap().width(),
                image::open(&first).unwrap().height()
            ),
            (3, 2)
        );
        let text = image_path_text(&first);
        assert!(text.starts_with('"') && text.ends_with("\" ") && text.contains("中文"));
        assert!(!text.contains(['\r', '\n', '\x1b']));
        assert!(save_image_png(b"invalid image", &directory).is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
        fs::remove_dir_all(directory).unwrap();
    }
}
