#[cfg(all(test, windows))]
#[path = "panel_workspace_native_test.rs"]
mod workspace_native_test;
use super::*;
use crate::{appearance::icon, project};
use gpui_kit::prelude::FluentBuilder;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum PanelTab {
    #[default]
    Explorer,
    Changes,
}
#[derive(Default)]
pub(super) struct ProjectPanel {
    pub open: bool,
    pub tab: PanelTab,
    workspace: Option<Uuid>,
    root: Option<PathBuf>,
    directories: HashMap<PathBuf, project::Directory>,
    expanded: HashSet<PathBuf>,
    pending: HashMap<PathBuf, Uuid>,
    errors: HashMap<PathBuf, String>,
    changes: Option<project::Changes>,
    changes_error: Option<String>,
    changes_request: Option<Uuid>,
    diff: Option<project::Diff>,
    diff_request: Option<Uuid>,
    selected: Option<(PathBuf, bool)>,
}
impl ProjectPanel {
    pub(super) fn for_workspace(data: &vibe_terminal::model::AppData) -> Self {
        Self {
            workspace: data.workspace().map(|workspace| workspace.id),
            open: data.workspace().is_some_and(|workspace| workspace.right_panel_open),
            ..Default::default()
        }
    }
}
impl Ui {
    pub(super) fn set_project_panel_open(&mut self, open: bool) {
        self.panel.open = open;
        if let Some(workspace) = self.session.data.workspace_mut() {
            workspace.right_panel_open = open;
        }
        self.session.persist();
        self.sync_project_panel();
    }
    pub(super) fn sync_project_panel(&mut self) {
        let workspace = self.session.data.workspace();
        let workspace_id = workspace.map(|workspace| workspace.id);
        let root = workspace
            .filter(|w| w.remote.is_none())
            .map(|w| w.directory.clone());
        if self.panel.workspace != workspace_id || self.panel.root != root {
            let open = if self.panel.workspace != workspace_id {
                workspace.is_some_and(|workspace| workspace.right_panel_open)
            } else {
                self.panel.open
            };
            self.panel = ProjectPanel {
                workspace: workspace_id,
                root: root.clone(),
                open,
                ..Default::default()
            };
        }
        if self.panel.open {
            if let Some(root) = root {
                if !self.panel.directories.contains_key(&root)
                    && !self.panel.pending.contains_key(&root)
                    && !self.panel.errors.contains_key(&root)
                {
                    self.load_directory(root);
                }
                if self.panel.tab == PanelTab::Changes
                    && self.panel.changes.is_none()
                    && self.panel.changes_error.is_none()
                    && self.panel.changes_request.is_none()
                {
                    self.load_changes();
                }
            }
        }
    }
    fn load_directory(&mut self, path: PathBuf) {
        let id = Uuid::new_v4();
        self.panel.pending.insert(path.clone(), id);
        self.panel.errors.remove(&path);
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = project::directory(&path);
            let _ = sender.send_blocking(Event::Directory(id, path, result));
        });
    }
    pub(super) fn accept_directory(
        &mut self,
        id: Uuid,
        path: PathBuf,
        result: Result<project::Directory>,
    ) {
        if self.panel.pending.get(&path) != Some(&id) {
            return;
        }
        self.panel.pending.remove(&path);
        match result {
            Ok(entries) => {
                self.panel.directories.insert(path, entries);
            }
            Err(error) => {
                self.panel.errors.insert(path, error.to_string());
            }
        }
    }
    fn load_changes(&mut self) {
        let Some(root) = self.panel.root.clone() else {
            return;
        };
        let id = Uuid::new_v4();
        self.panel.changes_request = Some(id);
        self.panel.changes_error = None;
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send_blocking(Event::Changes(id, project::changes(&root)));
        });
    }
    pub(super) fn accept_changes(&mut self, id: Uuid, result: Result<project::Changes>) {
        if self.panel.changes_request != Some(id) {
            return;
        }
        self.panel.changes_request = None;
        match result {
            Ok(changes) => self.panel.changes = Some(changes),
            Err(error) => self.panel.changes_error = Some(error.to_string()),
        }
    }
    pub(super) fn open_diff(&mut self, change: project::Change, staged: bool) {
        let Some(root) = self.panel.root.clone() else {
            return;
        };
        let id = Uuid::new_v4();
        self.panel.diff_request = Some(id);
        self.panel.selected = Some((change.path.clone(), staged));
        self.panel.diff = None;
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send_blocking(Event::Diff(id, project::diff(&root, &change, staged)));
        });
    }
    pub(super) fn accept_diff(&mut self, id: Uuid, result: Result<project::Diff>) {
        if self.panel.diff_request != Some(id) {
            return;
        }
        self.panel.diff_request = None;
        match result {
            Ok(diff) => self.panel.diff = Some(diff),
            Err(error) => self.session.error = Some(format!("无法读取差异：{error:#}")),
        }
    }
    fn tree_rows(&self, path: &PathBuf, depth: usize, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = div().flex().flex_col();
        if let Some(directory) = self.panel.directories.get(path) {
            for entry in &directory.entries {
                let path = entry.path.clone();
                let is_directory = entry.directory;
                let expanded = self.panel.expanded.contains(&path);
                let selected = self.active_document_matches(&path)
                    || (self.active_document.is_none()
                        && match &self.preview {
                            Some(PreviewContent::Code(code)) => {
                                vibe_terminal::paths::local_path(&code.file.path)
                                    == vibe_terminal::paths::local_path(&path)
                            }
                            Some(PreviewContent::Image(p)) => *p == path,
                            _ => false,
                        });
                let mut row = div()
                    .id(SharedString::from(format!("file-{}", path.display())))
                    .flex()
                    .items_center()
                    .h(px(28.0))
                    .gap_2()
                    .pr_2()
                    .pl(px(8.0 + depth as f32 * 16.0))
                    .rounded_sm()
                    .min_w_0()
                    .cursor_pointer()
                    .text_xs()
                    .bg(rgb(if selected { SURFACE } else { BG }))
                    .hover(|s| s.bg(rgb(SURFACE)))
                    .on_click(cx.listener(move |s, _, window, cx| {
                        if is_directory {
                            if !s.panel.expanded.remove(&path) {
                                s.panel.expanded.insert(path.clone());
                                if !s.panel.directories.contains_key(&path) {
                                    s.load_directory(path.clone());
                                }
                            }
                        } else {
                            s.open_document(
                                FileLink {
                                    start: 0,
                                    end: 0,
                                    path: path.clone(),
                                    line: None,
                                    column: None,
                                },
                                window,
                                cx,
                            );
                        }
                        cx.notify();
                    }));
                if is_directory {
                    row = row
                        .child(
                            icon(if expanded {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            })
                            .size(px(12.0)),
                        )
                        .child(icon("folder"));
                } else {
                    row = row.child(icon("file-code").text_color(rgb(MUTED)));
                }
                rows = rows.child(
                    row.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(entry.name.clone()),
                    ),
                );
                if expanded && depth < 20 {
                    rows = rows.child(self.tree_rows(&entry.path, depth + 1, cx));
                }
            }
            if directory.truncated {
                rows = rows.child(
                    div()
                        .p_2()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("此目录仅显示前 500 项"),
                );
            }
        } else if let Some(error) = self.panel.errors.get(path) {
            rows = rows.child(
                div()
                    .p_2()
                    .text_xs()
                    .text_color(rgb(0xefb3a9))
                    .child(error.clone()),
            );
        } else {
            rows = rows.child(
                div()
                    .p_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("正在读取目录…"),
            );
        }
        rows.into_any_element()
    }
    fn changes_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div()
            .id("changes-list")
            .flex_1()
            .flex()
            .flex_col()
            .p_2()
            .min_h_0()
            .overflow_y_scroll();
        if let Some(error) = &self.panel.changes_error {
            return list
                .child(
                    div()
                        .p_3()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(format!("无法读取 Git 状态：{error}")),
                )
                .into_any_element();
        }
        let Some(changes) = &self.panel.changes else {
            return list
                .child(div().p_3().text_xs().child("正在读取 Git 状态…"))
                .into_any_element();
        };
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .p_2()
                .text_xs()
                .child(icon("git-branch"))
                .child(changes.branch.clone()),
        );
        for (staged, label) in [(true, "暂存的更改"), (false, "更改")] {
            let entries: Vec<_> = changes
                .files
                .iter()
                .filter(|f| if staged { f.staged() } else { f.unstaged() })
                .collect();
            list = list.child(
                div()
                    .flex()
                    .justify_between()
                    .px_2()
                    .py_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(label)
                    .child(entries.len().to_string()),
            );
            if entries.is_empty() {
                list = list.child(
                    div()
                        .px_2()
                        .pb_2()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("暂无更改"),
                );
            }
            for file in entries {
                let change = file.clone();
                let selected = self
                    .panel
                    .selected
                    .as_ref()
                    .is_some_and(|(p, s)| *p == file.path && *s == staged);
                let status = if staged { file.index } else { file.worktree };
                list = list.child(
                    div()
                        .id(SharedString::from(format!(
                            "change-{staged}-{}",
                            file.relative
                        )))
                        .flex()
                        .items_center()
                        .gap_2()
                        .h(px(28.0))
                        .px_2()
                        .rounded_sm()
                        .min_w_0()
                        .bg(rgb(if selected { SURFACE } else { BG }))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(SURFACE)))
                        .on_click(cx.listener(move |s, _, _, cx| {
                            s.open_diff(change.clone(), staged);
                            cx.notify();
                        }))
                        .child(icon("file-code").text_color(rgb(MUTED)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .child(file.relative.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(if status == '?' { ACCENT } else { 0xc9b88a }))
                                .child(if status == '?' {
                                    "U".into()
                                } else {
                                    status.to_string()
                                }),
                        ),
                );
            }
        }
        if changes.truncated {
            list = list.child(div().p_2().text_xs().child("仅显示前 500 项更改"));
        }
        list.into_any_element()
    }
    fn diff_view(&self) -> AnyElement {
        let Some(diff) = &self.panel.diff else {
            return self.panel_empty(
                if self.panel.diff_request.is_some() {
                    "正在读取差异…"
                } else {
                    "选择更改查看差异"
                },
                "Git 工作区 · 只读差异预览",
            );
        };
        let mut lines = div().flex().flex_col();
        for (number, line) in diff.text.lines().take(2500).enumerate() {
            let (background, foreground) = if line.starts_with('+') && !line.starts_with("+++") {
                (0x202b21, 0xbbd4a8)
            } else if line.starts_with('-') && !line.starts_with("---") {
                (0x2c2222, 0xd4a6a0)
            } else {
                (BG, 0xbac7ac)
            };
            lines = lines.child(
                div()
                    .flex()
                    .min_w_full()
                    .bg(rgb(background))
                    .text_color(rgb(foreground))
                    .font_family("Geist Mono")
                    .text_xs()
                    .py_1()
                    .child(
                        div()
                            .w(px(40.0))
                            .flex_shrink_0()
                            .pr_2()
                            .text_align(TextAlign::Right)
                            .text_color(rgb(MUTED))
                            .child((number + 1).to_string()),
                    )
                    .child(div().whitespace_nowrap().pr_4().child(line.to_owned())),
            );
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                div()
                    .h(px(40.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .bg(self.surface_background(PANEL))
                    .child(icon("file-diff"))
                    .child(div().text_xs().truncate().child(display_path(&diff.path))),
            )
            .child(
                div()
                    .id("diff-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .child(lines),
            )
            .child(div().p_2().text_xs().text_color(rgb(MUTED)).child(
                if diff.truncated || diff.text.lines().count() > 2500 {
                    "差异较大，仅显示部分内容"
                } else {
                    "只读差异预览"
                },
            ))
            .into_any_element()
    }
    fn panel_empty(&self, title: &str, detail: &str) -> AnyElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .p_4()
            .text_xs()
            .text_color(rgb(MUTED))
            .child(icon("file-code").size(px(24.0)))
            .child(title.to_owned())
            .child(detail.to_owned())
            .into_any_element()
    }
    pub(super) fn right_panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut tabs = div()
            .flex()
            .items_center()
            .h(px(40.0))
            .flex_shrink_0()
            .bg(self.surface_background(PANEL));
        for (tab, name) in [
            (PanelTab::Explorer, "资源管理器"),
            (PanelTab::Changes, "源代码控制"),
        ] {
            let active = self.panel.tab == tab;
            tabs = tabs.child(
                div()
                    .id(SharedString::from(format!("panel-tab-{name}")))
                    .h_full()
                    .px_3()
                    .flex()
                    .items_center()
                    .text_xs()
                    .bg(self.surface_background(if active { BG } else { PANEL }))
                    .text_color(rgb(if active { FG } else { MUTED }))
                    .cursor_pointer()
                    .border_b_1()
                    .border_color(rgb(if active { ACCENT } else { SURFACE }))
                    .hover(|s| s.text_color(rgb(FG)))
                    .on_click(cx.listener(move |s, _, w, cx| {
                        s.panel.tab = tab;
                        s.focus.focus(w, cx);
                        s.sync_project_panel();
                        cx.notify();
                    }))
                    .child(name),
            );
        }
        tabs = tabs.child(div().flex_1()).child(Self::tool_button(
            "refresh-project",
            "刷新面板",
            gpui_kit::assets::IconName::RefreshCw,
            false,
            cx,
            |s, _, cx| {
                s.panel.directories.clear();
                s.panel.pending.clear();
                s.panel.errors.clear();
                s.panel.changes = None;
                s.panel.changes_error = None;
                s.panel.changes_request = None;
                s.panel.diff = None;
                s.panel.diff_request = None;
                s.panel.selected = None;
                s.sync_project_panel();
                cx.notify();
            },
        ));
        let mut pane = div()
            .flex()
            .flex_col()
            .w(px(self.preview_width))
            .h_full()
            .flex_shrink_0()
            .min_w_0()
            .bg(self.surface_background(BG))
            .child(tabs);
        let sidebar_preview = self.split_preview_owner.is_none()
            && (self.preview.is_some() || self.preview_request.is_some());
        if let Some(remote) = self
            .session
            .data
            .workspace()
            .and_then(|w| w.remote.as_ref())
        {
            if self.panel.tab == PanelTab::Explorer
                && matches!(self.preview, Some(PreviewContent::Web(_)))
            {
                return pane.child(self.preview(cx)).into_any_element();
            }
            return pane
                .child(self.panel_empty(
                    "SSH 远程工作区",
                    &format!(
                        "远端目录：{}。文件与 Git 操作请在 SSH 终端中进行。",
                        remote.path
                    ),
                ))
                .into_any_element();
        }
        if self.panel.tab == PanelTab::Changes {
            pane = pane
                .child(
                    div()
                        .h(px(
                            (f32::from(window.viewport_size().height) * 0.35).min(280.0)
                        ))
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .child(self.changes_list(cx)),
                )
                .child(div().h(px(1.0)).bg(rgb(SURFACE)))
                .child(self.diff_view());
        } else {
            if !sidebar_preview || !matches!(self.preview, Some(PreviewContent::Web(_))) {
                if let Some(root) = &self.panel.root {
                    pane = pane.child(
                        div()
                            .id("file-tree")
                            .when(sidebar_preview, |tree| {
                                tree.h(px(
                                    (f32::from(window.viewport_size().height) * 0.28).min(224.0)
                                ))
                                .flex_shrink_0()
                            })
                            .when(!sidebar_preview, |tree| tree.flex_1().min_h_0())
                            .overflow_y_scroll()
                            .p_2()
                            .child(
                                div().px_2().py_1().text_xs().text_color(rgb(MUTED)).child(
                                    self.session
                                        .data
                                        .workspace()
                                        .map(|w| w.name.clone())
                                        .unwrap_or_default(),
                                ),
                            )
                            .child(self.tree_rows(root, 0, cx)),
                    );
                }
                pane = pane.child(div().h(px(1.0)).bg(rgb(SURFACE)));
            }
            if sidebar_preview {
                pane = pane.child(self.preview(cx));
            } else if self.panel.root.is_none() {
                pane = pane.child(self.panel_empty("选择项目查看文件", "在左侧添加或选择一个项目"));
            }
        }
        pane.into_any_element()
    }
}
