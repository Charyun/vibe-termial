//! Image paste is captured only for the terminal or its conversation composer.
use super::*;
use vibe_terminal::clipboard::{
    ImagePaste, PasteTarget, image_paste, image_path_text, save_image_png,
};

pub(super) enum PasteDestination {
    Terminal,
    Composer { entity: EntityId, epoch: u64 },
}
impl Ui {
    pub(super) fn try_paste_image(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.settings || self.dialog.is_some() || self.active_document.is_some() {
            return false;
        }
        let destination = if let Some((entity, epoch)) = self.image_composer_target(window, cx) {
            PasteDestination::Composer { entity, epoch }
        } else if !self.content_visible() && self.focus.is_focused(window) {
            PasteDestination::Terminal
        } else {
            return false;
        };
        let Some(clipboard) = cx.read_from_clipboard() else {
            return false;
        };
        let Some(id) = self.session.selected() else {
            return false;
        };
        self.paste_clipboard_image(id, destination, &clipboard, cx)
    }
    pub(super) fn paste_clipboard_image(
        &mut self,
        id: Uuid,
        destination: PasteDestination,
        clipboard: &ClipboardItem,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.settings
            || self.dialog.is_some()
            || self.active_document.is_some()
            || self.ssh.modal.is_some()
            || self.session.selected() != Some(id)
        {
            return false;
        }
        let Some(image) = clipboard.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image),
            _ => None,
        }) else {
            return false;
        };
        if let PasteDestination::Composer { entity, epoch } = &destination
            && !self.composer_image_destination_current(id, *entity, *epoch)
        {
            return true;
        }
        if self.session.data.is_remote_tab(id) {
            self.session.error =
                Some("SSH 终端暂不支持直接粘贴本机图片，请先把图片上传到远端。".into());
            cx.notify();
            return true;
        }
        let target = match self.session.capture_paste_target(id) {
            Ok(target) => target,
            Err(error) => {
                self.session.error = Some(format!("无法粘贴图片：{error:#}"));
                cx.notify();
                return true;
            }
        };
        let Some(tab) = self.session.data.tab(id) else {
            return true;
        };
        match image_paste(&tab.profile) {
            ImagePaste::Native(bytes) => match self.session.send_paste_shortcut(&target, bytes) {
                Ok(()) => {
                    self.native_images.insert(id, target);
                    self.native_image_notice(id, cx);
                }
                Err(error) => self.session.error = Some(format!("无法粘贴图片：{error:#}")),
            },
            ImagePaste::FilePath => {
                if matches!(destination, PasteDestination::Composer { .. }) {
                    self.begin_composer_image_paste(id, cx);
                }
                let bytes = image.bytes().to_vec();
                let sender = self.sender.clone();
                std::thread::spawn(move || {
                    let directory = std::env::temp_dir()
                        .join("vibe-terminal")
                        .join("clipboard-images");
                    let result = save_image_png(&bytes, &directory);
                    let event = Event::ClipboardImage(target, destination, result);
                    if let Err(error) = sender.send_blocking(event)
                        && let Event::ClipboardImage(_, _, Ok(path)) = error.0
                    {
                        let _ = std::fs::remove_file(path);
                    }
                });
            }
        }
        cx.notify();
        true
    }
    pub(super) fn accept_clipboard_image(
        &mut self,
        target: PasteTarget,
        destination: PasteDestination,
        result: Result<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = target.tab_id();
        if let PasteDestination::Composer { entity, .. } = &destination {
            self.finish_composer_image_paste(id, *entity, cx);
        }
        let path = match result {
            Ok(path) => path,
            Err(error) => {
                self.clipboard_image_error(id, format!("图片粘贴失败：{error:#}"), cx);
                return;
            }
        };
        if !self.session.paste_target_current(&target) {
            let _ = std::fs::remove_file(path);
            if self.session.selected() == Some(id) {
                self.clipboard_image_error(id, "会话已更换，请重新粘贴图片".into(), cx);
            }
            return;
        }
        match destination {
            PasteDestination::Terminal => {
                if let Err(error) = self.session.paste_image_path(&target, &path) {
                    let _ = std::fs::remove_file(&path);
                    self.clipboard_image_error(id, format!("图片路径粘贴失败：{error:#}"), cx);
                }
            }
            PasteDestination::Composer { entity, epoch } => {
                if !self.insert_composer_image_path(
                    id,
                    entity,
                    epoch,
                    image_path_text(&path),
                    window,
                    cx,
                ) {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        cx.notify();
    }
}
