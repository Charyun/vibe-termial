use super::*;

impl Ui {
    pub(super) fn dropped_paths_text(
        &mut self,
        paths: &ExternalPaths,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if paths.paths().is_empty()
            || self.settings
            || self.dialog.is_some()
            || self.ssh.modal.is_some()
            || self.active_document.is_some()
        {
            return None;
        }
        let id = self.session.selected()?;
        if self.session.data.is_remote_tab(id) {
            self.session.error =
                Some("SSH 终端无法直接访问本机拖入的文件，请先将文件上传到远端。".into());
            cx.notify();
            return None;
        }
        if !self
            .session
            .terminals
            .get(&id)
            .is_some_and(|terminal| terminal.running())
        {
            return None;
        }
        let profile = &self.session.data.tab(id)?.profile;
        match vibe_terminal::file_drop::path_input(profile, paths.paths()) {
            Ok(text) => Some(text),
            Err(error) => {
                self.session.error = Some(format!("无法拖入文件：{error:#}"));
                cx.notify();
                None
            }
        }
    }

    pub(super) fn drop_terminal_files(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.content_visible() {
            return;
        }
        if let Some(text) = self.dropped_paths_text(paths, cx) {
            self.focus.focus(window, cx);
            self.session.paste(&text);
            self.selection = None;
            self.path_press = None;
            self.selecting = false;
            cx.notify();
        }
    }
}
