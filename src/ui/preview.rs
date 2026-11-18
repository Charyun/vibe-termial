//! File preview modes share the existing editor and save/conflict handling.
#[cfg(all(test, windows))]
#[path = "preview_native_test.rs"]
mod native_test;
use super::*;

impl Ui {
    pub(super) fn markdown_preview(&self) -> Option<&CodePreview> {
        if self.preview_editing {
            return None;
        }
        let Some(PreviewContent::Code(code)) = &self.preview else {
            return None;
        };
        let extension = code.file.path.extension()?.to_str()?;
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "md" | "markdown" | "mdown" | "mkd" | "mdwn"
        )
        .then_some(code)
    }
    pub(super) fn toggle_preview_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preview_editing && !self.save_preview(cx) {
            return;
        }
        self.preview_editing = !self.preview_editing;
        if self.markdown_preview().is_some() {
            // The source editor is no longer mounted in the Markdown preview.
            self.focus.focus(window, cx);
        } else if let Some(input) = &self.preview_editor {
            input.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }
}
