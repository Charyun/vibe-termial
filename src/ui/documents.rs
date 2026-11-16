//! Main-area file tabs are independent from terminal-owned split previews.
#[cfg(all(test, windows))]
#[path = "documents_native_test.rs"]
mod native_test;
use super::*;

pub(super) enum DocumentContent {
    Code(CodePreview),
    Image(PathBuf),
}
pub(super) struct DocumentTab {
    pub id: Uuid,
    pub workspace: Uuid,
    pub path: PathBuf,
    pub title: String,
    pub dirty: bool,
    request: Option<Uuid>,
    content: Option<DocumentContent>,
    editor: Option<Entity<EditorState>>,
    subscription: Option<Subscription>,
    editing: bool,
    wrap: bool,
    error: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MainTab {
    Terminal(Uuid),
    Document(Uuid),
}

fn canonical_file(path: &std::path::Path) -> PathBuf {
    vibe_terminal::paths::local_path(&path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
}
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}
fn read_document(link: FileLink) -> Result<DocumentContent> {
    let extension = link
        .path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
    ) {
        if std::fs::metadata(&link.path)?.len() > 16 * 1024 * 1024 {
            bail!("图片超过 16 MB")
        }
        let (width, height) = image::ImageReader::open(&link.path)?
            .with_guessed_format()?
            .into_dimensions()?;
        if width as u64 * height as u64 > 16_000_000 {
            bail!("图片超过 1600 万像素")
        }
        Ok(DocumentContent::Image(link.path))
    } else {
        load_code_preview(link).map(DocumentContent::Code)
    }
}
impl Ui {
    pub(super) fn open_document(
        &mut self,
        mut link: FileLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.session.data.selected_workspace else {
            return;
        };
        link.path = canonical_file(&link.path);
        if let Some(id) = self
            .documents
            .iter()
            .find(|tab| tab.workspace == workspace && same_file(&tab.path, &link.path))
            .map(|tab| tab.id)
        {
            self.activate_document(id, window, cx);
            return;
        }
        let id = Uuid::new_v4();
        let request = Uuid::new_v4();
        let title = link
            .path
            .file_name()
            .unwrap_or(link.path.as_os_str())
            .to_string_lossy()
            .into_owned();
        self.documents.push(DocumentTab {
            id,
            workspace,
            path: link.path.clone(),
            title,
            dirty: false,
            request: Some(request),
            content: None,
            editor: None,
            subscription: None,
            editing: false,
            wrap: false,
            error: None,
        });
        self.activate_document(id, window, cx);
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send_blocking(Event::Document(id, request, read_document(link)));
        });
    }
    pub(super) fn activate_document(
        &mut self,
        id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self
            .documents
            .iter()
            .find(|tab| tab.id == id)
            .map(|tab| tab.workspace)
        else {
            return;
        };
        if self.session.data.selected_workspace != Some(workspace) {
            self.select_workspace(workspace);
        }
        self.active_document = Some(id);
        self.settings = false;
        self.clear_mouse_hover();
        self.focus_current_view(window, cx);
        cx.notify();
    }
    pub(super) fn focus_document(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(tab) = self
            .active_document
            .and_then(|id| self.documents.iter().find(|tab| tab.id == id))
        {
            if let Some(editor) = &tab.editor {
                editor.update(cx, |editor, cx| editor.focus(window, cx));
            } else {
                self.focus.focus(window, cx);
            }
            true
        } else {
            false
        }
    }
    pub(super) fn active_document_matches(&self, path: &std::path::Path) -> bool {
        self.active_document
            .and_then(|id| self.documents.iter().find(|tab| tab.id == id))
            .is_some_and(|tab| same_file(&tab.path, &canonical_file(path)))
    }
    pub(super) fn accept_document(
        &mut self,
        id: Uuid,
        request: Uuid,
        result: Result<DocumentContent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .documents
            .iter()
            .position(|tab| tab.id == id && tab.request == Some(request))
        else {
            return;
        };
        self.documents[index].request = None;
        let content = match result {
            Ok(content) => content,
            Err(error) => {
                self.documents[index].error = Some(format!("无法打开文件：{error:#}"));
                cx.notify();
                return;
            }
        };
        if let DocumentContent::Code(code) = &content {
            let language = code
                .file
                .path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("text")
                .to_owned();
            let editor = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language(language)
                    .default_value(code.file.content.clone())
                    .soft_wrap(false)
                    .line_number(true)
                    .folding(false)
            });
            let line = code
                .file
                .line
                .unwrap_or(1)
                .saturating_sub(1)
                .min(code.file.content.lines().count().saturating_sub(1))
                as u32;
            let column = code.file.column.unwrap_or(1).saturating_sub(1) as u32;
            editor.update(cx, |editor, cx| {
                editor.set_cursor_position(Position::new(line, column), window, cx)
            });
            let subscription = cx.subscribe_in(&editor, window, move |ui, state, event, _, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(tab) = ui.documents.iter_mut().find(|tab| tab.id == id)
                    && let Some(DocumentContent::Code(code)) = &tab.content
                {
                    tab.dirty = state.read(cx).value().replace("\r\n", "\n")
                        != code.file.content.replace("\r\n", "\n");
                    cx.notify();
                }
            });
            self.documents[index].editor = Some(editor);
            self.documents[index].subscription = Some(subscription);
        }
        self.documents[index].content = Some(content);
        if self.active_document == Some(id) && !self.settings && self.dialog.is_none() {
            self.focus_current_view(window, cx);
        }
        cx.notify();
    }
    pub(super) fn save_document(&mut self, id: Uuid, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == id) else {
            return true;
        };
        let Some(editor) = &tab.editor else {
            return true;
        };
        let text = editor.read(cx).value().to_string();
        if let Some(DocumentContent::Code(code)) = &mut tab.content {
            match code.file.save(&text) {
                Ok(()) => {
                    tab.dirty = false;
                    tab.error = None;
                    cx.notify();
                    true
                }
                Err(error) => {
                    tab.error = Some(format!("保存失败：{error:#}"));
                    cx.notify();
                    false
                }
            }
        } else {
            true
        }
    }
    pub(super) fn request_close_document(
        &mut self,
        id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.documents.iter().any(|tab| tab.id == id && tab.dirty) {
            self.activate_document(id, window, cx);
            self.pending_document = Some(id);
            self.pending_preview = Some(PreviewAction::CloseDocument(id));
            self.open_dialog(Dialog::UnsavedPreview, window, cx);
            cx.notify();
        } else {
            self.close_document(id, window, cx);
        }
    }
    pub(super) fn close_document(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.documents.iter().position(|tab| tab.id == id) else {
            return;
        };
        let owner = self.documents[index].workspace;
        self.documents.remove(index);
        if self.active_document == Some(id) {
            self.active_document = self
                .documents
                .iter()
                .filter(|tab| tab.workspace == owner)
                .last()
                .map(|tab| tab.id);
            self.focus_current_view(window, cx);
        }
        cx.notify();
    }
    /// Ask about every dirty document before quitting or removing its workspace.
    pub(super) fn guard_dirty_documents(
        &mut self,
        workspace: Option<Uuid>,
        action: PreviewAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let id = self
            .documents
            .iter()
            .find(|tab| tab.dirty && workspace.is_none_or(|owner| owner == tab.workspace))
            .map(|tab| tab.id);
        if let Some(id) = id {
            self.activate_document(id, window, cx);
            self.pending_document = Some(id);
            self.pending_preview = Some(action);
            self.open_dialog(Dialog::UnsavedPreview, window, cx);
            cx.notify();
            true
        } else {
            false
        }
    }
    pub(super) fn main_tabs(&self) -> Vec<MainTab> {
        let Some(workspace) = self.session.data.workspace() else {
            return vec![];
        };
        workspace
            .tabs
            .iter()
            .map(|tab| MainTab::Terminal(tab.id))
            .chain(
                self.documents
                    .iter()
                    .filter(|tab| tab.workspace == workspace.id)
                    .map(|tab| MainTab::Document(tab.id)),
            )
            .collect()
    }
    pub(super) fn current_main_tab(&self) -> Option<MainTab> {
        self.active_document
            .map(MainTab::Document)
            .or_else(|| self.session.selected().map(MainTab::Terminal))
    }
    pub(super) fn select_main_tab(
        &mut self,
        target: MainTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            MainTab::Terminal(id) => self.select_tab(id, window, cx),
            MainTab::Document(id) => self.activate_document(id, window, cx),
        }
    }
    pub(super) fn terminal_split_visible(&self) -> bool {
        self.active_document.is_none()
            && self.split_preview_owner.is_some()
            && self.split_preview_owner == self.session.selected()
            && (self.preview.is_some() || self.preview_request.is_some())
    }
    pub(super) fn document_tab_buttons(
        &self,
        workspace: Uuid,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        self.documents
            .iter()
            .filter(|tab| tab.workspace == workspace)
            .map(|tab| {
                let id = tab.id;
                let active = self.active_document == Some(id);
                let group: SharedString = format!("document-tab-hover-{id}").into();
                div()
                    .id(SharedString::from(format!("document-tab-container-{id}")))
                    .group(group.clone())
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex()
                    .items_center()
                    .h_full()
                    .flex_shrink_0()
                    .relative()
                    .bg(rgb(if active { BG } else { PANEL }))
                    .border_t_1()
                    .border_color(rgb(if active { 0x8b9c89 } else { PANEL }))
                    .child(Self::tab_separator())
                    .child(
                        div()
                            .id(SharedString::from(format!("document-tab-{id}")))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .h_full()
                            .max_w(px(224.0))
                            .min_w_0()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(rgb(if active { FG } else { MUTED }))
                            .hover(|style| style.bg(rgb(SURFACE)))
                            .on_click(cx.listener(move |ui, _, window, cx| {
                                ui.activate_document(id, window, cx)
                            }))
                            .child(icon("file-code"))
                            .child(div().truncate().child(format!(
                                "{}{}",
                                tab.title,
                                if tab.dirty { " *" } else { "" }
                            ))),
                    )
                    .child(
                        div()
                            .invisible()
                            .group_hover(group, |style| style.visible())
                            .child(
                                Self::tool_button(
                                    SharedString::from(format!("close-document-{id}")),
                                    "关闭文件",
                                    gpui_kit::assets::IconName::X,
                                    false,
                                    cx,
                                    move |ui, window, cx| ui.request_close_document(id, window, cx),
                                )
                                .size(px(18.0)),
                            ),
                    )
                    .into_any_element()
            })
            .collect()
    }
    pub(super) fn main_area(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut area = div()
            .id("main-content-area")
            .flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(if self.active_document.is_some() {
                self.document_view(cx)
            } else {
                self.terminal(cx)
            });
        if self.terminal_split_visible() {
            let available = f32::from(window.viewport_size().width)
                - SIDEBAR_WIDTH
                - if self.panel.open {
                    self.preview_width + 4.0
                } else {
                    0.0
                };
            let width = self
                .terminal_preview_width
                .clamp(120.0, (available - 160.0).max(120.0));
            area = area
                .child(
                    div()
                        .id("terminal-preview-divider")
                        .w(px(4.0))
                        .h_full()
                        .flex_shrink_0()
                        .bg(rgb(SURFACE))
                        .hover(|style| style.bg(rgb(ACCENT)))
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|ui, event: &MouseDownEvent, _, _| {
                                ui.resizing_terminal_preview = true;
                                ui.resize_origin =
                                    Some((event.position.x, ui.terminal_preview_width));
                            }),
                        ),
                )
                .child(
                    div()
                        .id("terminal-file-preview")
                        .w(px(width))
                        .h_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .child(self.preview(cx)),
                );
        }
        area.into_any_element()
    }
    pub(super) fn document_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(tab) = self
            .active_document
            .and_then(|id| self.documents.iter().find(|tab| tab.id == id))
        else {
            return div().into_any_element();
        };
        let id = tab.id;
        let mut toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .flex_shrink_0()
            .bg(rgb(PANEL))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(display_path(&tab.path)),
            );
        if tab.editor.is_some() {
            toolbar = toolbar
                .child(Self::button(
                    "document-wrap",
                    if tab.wrap {
                        "换行：开"
                    } else {
                        "换行：关"
                    },
                    cx,
                    move |ui, window, cx| {
                        if let Some(tab) = ui.documents.iter_mut().find(|tab| tab.id == id) {
                            tab.wrap = !tab.wrap;
                            if let Some(editor) = &tab.editor {
                                editor.update(cx, |editor, cx| {
                                    editor.set_soft_wrap(tab.wrap, window, cx)
                                });
                            }
                        }
                    },
                ))
                .child(Self::button(
                    "document-edit",
                    if tab.editing { "完成" } else { "编辑" },
                    cx,
                    move |ui, window, cx| {
                        let finishing = ui.documents.iter().any(|tab| tab.id == id && tab.editing);
                        if finishing && !ui.save_document(id, cx) {
                            return;
                        }
                        if let Some(tab) = ui.documents.iter_mut().find(|tab| tab.id == id) {
                            tab.editing = !tab.editing;
                        }
                        ui.focus_current_view(window, cx);
                    },
                ))
                .child(Self::button(
                    "document-save",
                    "保存",
                    cx,
                    move |ui, _, cx| {
                        ui.save_document(id, cx);
                    },
                ));
        }
        toolbar = toolbar.child(Self::button(
            "document-copy",
            if tab.editor.is_some() {
                "复制"
            } else {
                "复制路径"
            },
            cx,
            move |ui, _, cx| {
                if let Some(tab) = ui.documents.iter().find(|tab| tab.id == id) {
                    let text = if let Some(editor) = &tab.editor {
                        let editor = editor.read(cx);
                        let selection = editor.selected_value();
                        if selection.is_empty() {
                            editor.value().to_string()
                        } else {
                            selection.to_string()
                        }
                    } else {
                        display_path(&tab.path)
                    };
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            },
        ));
        let mut pane = div()
            .id("document-view")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .bg(rgb(BG))
            .child(toolbar);
        if let Some(error) = &tab.error {
            pane = pane.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(rgb(0xefb3a9))
                    .child(error.clone()),
            );
        }
        match &tab.content {
            Some(DocumentContent::Code(_)) => {
                if let Some(editor) = &tab.editor {
                    pane = pane.child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .font_family("Geist Mono")
                            .text_size(px(12.0))
                            .child(
                                CodeEditor::new(editor)
                                    .readonly(!tab.editing)
                                    .h(relative(1.0))
                                    .bordered(false)
                                    .appearance(false),
                            ),
                    );
                }
                pane = pane.child(div().px_3().py_1().text_xs().text_color(rgb(MUTED)).child(
                    if tab.dirty {
                        "未保存 · Ctrl+S 保存"
                    } else if tab.editing {
                        "可编辑 · Ctrl+S 保存"
                    } else {
                        "只读预览 · 点击编辑可修改"
                    },
                ));
            }
            Some(DocumentContent::Image(path)) => {
                pane = pane.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .p_4()
                        .child(img(path.clone()).size_full().object_fit(ObjectFit::Contain)),
                )
            }
            None => {
                if tab.request.is_some() {
                    pane = pane.child(
                        div()
                            .p_4()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child("正在读取文件…"),
                    );
                }
            }
        }
        pane.into_any_element()
    }
}
