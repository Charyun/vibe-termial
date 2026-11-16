#[cfg(test)]
mod acceptance;
mod activity;
mod chrome;
mod clipboard;
mod conversation;
mod documents;
mod panels;
mod ssh;
use crate::appearance::{control_icon, icon, profile_icon, profile_icon_sized};
use crate::render::{self, CELL, INSET, LINE, Snapshot};
use anyhow::{Result, bail};
use gpui_kit::{
    component::{
        Disableable, Selectable, Sizable,
        button::*,
        input::{Editor as CodeEditor, EditorState, Input, InputEvent, InputState, Position},
    },
    *,
};
use panels::{PanelTab, ProjectPanel};
use std::{
    cell::RefCell,
    collections::HashMap,
    ops::Range,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;
use vibe_terminal::{
    model::{CliKind, CliProfile, InputCursorStyle},
    paths::{FileLink, Preview},
    scroll::WheelAccumulator,
    session::Session,
};

const BG: u32 = 0x181818;
const PANEL: u32 = 0x1f1f1f;
const SURFACE: u32 = 0x272727;
const FG: u32 = 0xe6e7e6;
const MUTED: u32 = 0x90978f;
const ACCENT: u32 = 0xb4edc5;
const SIDEBAR_WIDTH: f32 = 224.0;

type CellPoint = (usize, usize);
#[derive(Clone)]
enum Dialog {
    RenameWorkspace(Uuid),
    RenameTab(Uuid),
    RemoveWorkspace(Uuid),
    CloseTab(Uuid),
    UnsavedPreview,
}
#[derive(Clone)]
enum PreviewAction {
    Close,
    CloseDocument(Uuid),
    File(FileLink),
    Web(String),
    Workspace(Uuid),
    AddWorkspace(PathBuf),
    AddRemoteWorkspace {
        host_id: Uuid,
        path: String,
        name: String,
    },
    RemoveWorkspace(Uuid),
    Quit,
}
enum Editor {
    Single(Entity<InputState>),
}
enum PreviewContent {
    Code(CodePreview),
    Image(PathBuf),
    Web(WebPreview),
}
struct WebPreview {
    id: Uuid,
    url: String,
    error: Option<String>,
}
struct CodePreview {
    file: Preview,
}

#[derive(Clone, Copy, PartialEq)]
enum SettingsTab {
    General,
    Agents,
    Shortcuts,
    Remote,
    SshHosts,
}
impl SettingsTab {
    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Agents => "Agent",
            Self::Shortcuts => "快捷键",
            Self::Remote => "Remote",
            Self::SshHosts => "SSH 主机",
        }
    }
}
enum Event {
    Output,
    SshProbe(Uuid, Uuid, Result<String>),
    Preview(Uuid, Result<PreviewContent>),
    Document(Uuid, Uuid, Result<documents::DocumentContent>),
    ClipboardImage(
        vibe_terminal::clipboard::PasteTarget,
        clipboard::PasteDestination,
        Result<PathBuf>,
    ),
    Web(vibe_terminal::web::Request),
    TailscaleAddress(Result<String>),
    BrowserNavigated(Uuid, String),
    BrowserError(Uuid, String),
    BrowserReady(Uuid),
    Directory(Uuid, PathBuf, Result<crate::project::Directory>),
    Changes(Uuid, Result<crate::project::Changes>),
    Diff(Uuid, Result<crate::project::Diff>),
}

pub struct Ui {
    pub session: Session,
    focus: FocusHandle,
    snapshot: Arc<Snapshot>,
    snapshot_cache: render::SnapshotCache,
    text_cache: RefCell<render::TextCache>,
    running_terminals: std::collections::HashSet<Uuid>,
    agent_activity: HashMap<Uuid, vibe_terminal::activity::Indicator>,
    bounds: Bounds<Pixels>,
    selection: Option<(CellPoint, CellPoint)>,
    selecting: bool,
    mouse_capture: Option<(Uuid, CellPoint)>,
    mouse_hover: Option<(Uuid, CellPoint)>,
    terminal_pointer: Option<Point<Pixels>>,
    path_press: Option<(CellPoint, Point<Pixels>)>,
    preedit: String,
    last_ime_geometry: Option<(Bounds<Pixels>, f32)>,
    settings: bool,
    settings_tab: SettingsTab,
    expanded_workspaces: Vec<Uuid>,
    conversations: HashMap<Uuid, conversation::ConversationView>,
    documents: Vec<documents::DocumentTab>,
    native_images: HashMap<Uuid, vibe_terminal::clipboard::PasteTarget>,
    active_document: Option<Uuid>,
    pending_document: Option<Uuid>,
    detected_agents: Vec<vibe_terminal::agents::Detected>,
    wheel: WheelAccumulator,
    dialog: Option<Dialog>,
    editor: Option<Editor>,
    preview: Option<PreviewContent>,
    split_preview_owner: Option<Uuid>,
    terminal_preview_width: f32,
    resizing_terminal_preview: bool,
    panel: ProjectPanel,
    ssh: ssh::SshUi,
    browser: Rc<RefCell<crate::browser::Browser>>,
    preview_request: Option<Uuid>,
    preview_editor: Option<Entity<EditorState>>,
    preview_subscription: Option<Subscription>,
    preview_dirty: bool,
    preview_editing: bool,
    preview_wrap: bool,
    pending_preview: Option<PreviewAction>,
    preview_width: f32,
    resize_origin: Option<(Pixels, f32)>,
    sender: async_channel::Sender<Event>,
    web_server: Option<vibe_terminal::web::Server>,
    web_tailscale: Option<String>,
    new_menu: Option<(f32, f32)>,
    _tasks: Vec<Task<()>>,
    pub demo: bool,
}
impl Ui {
    pub fn new(mut session: Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        session.ensure_history();
        let (sender, receiver) = async_channel::bounded(64);
        let queued = Arc::new(AtomicBool::new(false));
        let wake_sender = sender.clone();
        let wake_queued = queued.clone();
        session.wake = Arc::new(move || {
            if !wake_queued.swap(true, Ordering::AcqRel)
                && wake_sender.try_send(Event::Output).is_err()
            {
                wake_queued.store(false, Ordering::Release);
            }
        });
        let events = cx.spawn_in(window, async move |weak, cx| {
            while let Ok(event) = receiver.recv().await {
                let output = matches!(event, Event::Output);
                if output {
                    queued.store(false, Ordering::Release);
                }
                if weak
                    .update_in(cx, |ui, window, cx| {
                        match event {
                            Event::Output => {
                                // Always drain every flag, even when only a hidden PTY changed.
                                ui.session.take_dirty();
                                let changed = ui.refresh_snapshot(false);
                                let running = ui.session.terminals.iter()
                                    .filter(|(_, terminal)| terminal.running())
                                    .map(|(id, _)| *id)
                                    .collect();
                                let status_changed = ui.running_terminals != running;
                                ui.running_terminals = running;
                                let activity_changed = ui.refresh_agent_activity();
                                if !changed && !status_changed && !activity_changed {
                                    return;
                                }
                            }
                            Event::Web(request) => {
                                if !request.reply.is_closed() {
                                    let result = ui
                                        .session
                                        .web_command(request.command)
                                        .map_err(|e| format!("{e:#}"));
                                    let _ = request.reply.send(result);
                                    ui.refresh();
                                }
                            }
                            Event::TailscaleAddress(result) => match result {
                                Ok(address) => ui.web_tailscale = Some(address),
                                Err(error) => {
                                    ui.session.error =
                                        Some(format!("Tailscale 地址获取失败：{error:#}"))
                                }
                            },
                            Event::SshProbe(request,host,result) => ui.accept_ssh_probe(request,host,result),
                            Event::Directory(id, path, result) => ui.accept_directory(id, path, result),
                            Event::ClipboardImage(target, destination, result) => ui.accept_clipboard_image(target, destination, result, window, cx),
                            Event::Document(id, request, result) => ui.accept_document(id, request, result, window, cx),
                            Event::Changes(id, result) => ui.accept_changes(id, result),
                            Event::Diff(id, result) => ui.accept_diff(id, result),
                            Event::BrowserNavigated(id, url) => {
                                if let Some(PreviewContent::Web(web)) = &mut ui.preview
                                    && web.id == id
                                {
                                    web.url = url;
                                }
                            }
                            Event::BrowserReady(id) => {
                                if !matches!(&ui.preview, Some(PreviewContent::Web(web)) if web.id == id) {
                                    return;
                                }
                            }
                            Event::BrowserError(id, error) => {
                                if let Some(PreviewContent::Web(web)) = &mut ui.preview
                                    && web.id == id
                                {
                                    web.error = Some(error);
                                }
                            }
                            Event::Preview(id, result) if ui.preview_request == Some(id) => {
                                ui.preview_request = None;
                                match result {
                                    Ok(preview) => ui.install_preview(preview, window, cx),
                                    Err(e) => ui.session.error = Some(format!("预览失败：{e:#}")),
                                }
                            }
                            _ => {}
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
                if output {
                    smol::Timer::after(Duration::from_millis(33)).await;
                }
            }
        });
        let checkpoint = cx.spawn(async move |weak, cx| {
            let mut seconds = 0;
            loop {
                smol::Timer::after(Duration::from_secs(2)).await;
                seconds += 2;
                if weak
                    .update(cx, |ui, _| {
                        if !ui.demo {
                            ui.session.capture_metadata();
                            if seconds % 10 == 0
                                && ui.session.terminals.values().any(|t| t.loaded())
                            {
                                ui.session.checkpoint();
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let title_root = session.store.root.clone();
        let titles = cx.spawn(async move |weak, cx| {
            let mut tracker = vibe_terminal::titles::TitleTracker::default();
            loop {
                let Ok(tabs) = weak.update(cx, |ui, _| {
                    ui.session
                        .data
                        .workspaces
                        .iter()
                        .flat_map(|workspace| &workspace.tabs)
                        .filter(|tab| !ui.demo && tab.accepts_session_summary())
                        .cloned()
                        .collect::<Vec<_>>()
                }) else {
                    break;
                };
                let root = title_root.clone();
                let (next, updates) = smol::unblock(move || {
                    let updates = tracker.refresh(&tabs, &root);
                    (tracker, updates)
                })
                .await;
                tracker = next;
                if weak
                    .update(cx, |ui, cx| {
                        let mut changed = false;
                        for update in updates {
                            if let Some(tab) = ui.session.data.tab_mut(update.tab_id) {
                                changed |= update.apply(tab);
                            }
                        }
                        if changed {
                            ui.session.persist();
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
                smol::Timer::after(Duration::from_secs(3)).await;
            }
        });
        let conversations = Self::spawn_conversation_reader(cx);
        let activity = Self::spawn_agent_activity_reader(cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let expanded_workspaces = session.data.workspaces.iter().map(|w| w.id).collect();
        let mut ui = Self {
            session,
            focus,
            snapshot: Arc::default(),
            snapshot_cache: render::SnapshotCache::default(),
            text_cache: RefCell::default(),
            running_terminals: std::collections::HashSet::new(),
            agent_activity: HashMap::new(),
            bounds: Bounds::default(),
            selection: None,
            selecting: false,
            mouse_capture: None,
            mouse_hover: None,
            terminal_pointer: None,
            path_press: None,
            preedit: String::new(),
            last_ime_geometry: None,
            settings: false,
            settings_tab: SettingsTab::General,
            expanded_workspaces,
            conversations: HashMap::new(),
            documents: vec![],
            native_images: HashMap::new(),
            active_document: None,
            pending_document: None,
            detected_agents: Vec::new(),
            new_menu: None,
            wheel: WheelAccumulator::default(),
            dialog: None,
            editor: None,
            preview: None,
            split_preview_owner: None,
            terminal_preview_width: 360.0,
            resizing_terminal_preview: false,
            panel: ProjectPanel::default(),
            ssh: ssh::SshUi::default(),
            browser: Rc::new(RefCell::new(crate::browser::Browser::new())),
            preview_request: None,
            preview_editor: None,
            preview_subscription: None,
            preview_dirty: false,
            preview_editing: false,
            preview_wrap: true,
            pending_preview: None,
            preview_width: 440.0,
            resize_origin: None,
            sender,
            web_server: None,
            web_tailscale: None,
            _tasks: vec![events, checkpoint, titles, conversations, activity],
            demo: false,
        };
        ui.refresh();
        // 启动时只恢复尚未加载的选中会话。
        if let Some(id) = ui.session.selected()
            && !ui
                .session
                .terminals
                .get(&id)
                .is_some_and(|terminal| terminal.loaded())
        {
            ui.session.restore(id);
            ui.refresh();
        }
        ui
    }
    fn toggle_web(&mut self, cx: &mut Context<Self>) {
        if self.web_server.take().is_some() {
            self.web_tailscale = None;
            cx.notify();
            return;
        }
        let sender = self.sender.clone();
        let dispatch = Arc::new(move |request| {
            sender
                .try_send(Event::Web(request))
                .map_err(|_| "桌面正忙或已关闭".to_owned())
        });
        let address =
            std::env::var("VIBE_TERMINAL_WEB_BIND").unwrap_or_else(|_| "0.0.0.0:8787".into());
        let result = address
            .parse()
            .map_err(anyhow::Error::from)
            .and_then(|address| vibe_terminal::web::Server::start(address, dispatch));
        match result {
            Ok(server) => {
                cx.write_to_clipboard(ClipboardItem::new_string(server.lan_url()));
                self.web_server = Some(server);
                self.session.error = None;
            }
            Err(error) => self.session.error = Some(format!("Web 服务启动失败：{error:#}")),
        }
        cx.notify();
    }
    fn find_tailscale(&mut self, cx: &mut Context<Self>) {
        let sender = self.sender.clone();
        cx.spawn(async move |_, _| {
            let result = smol::unblock(|| -> Result<String> {
                let mut command = vibe_terminal::tailscale::command();
                command.args(["ip", "-4"]);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    command.creation_flags(0x08000000);
                }
                let output = command.output().map_err(|error| match error.kind() {
                    std::io::ErrorKind::NotFound => anyhow::anyhow!(
                        "未找到 tailscale 命令，请确认已安装 Tailscale，或把它的安装目录加入 PATH"
                    ),
                    _ => anyhow::Error::from(error),
                })?;
                if !output.status.success() {
                    let detail = String::from_utf8_lossy(&output.stderr);
                    let detail = detail.trim();
                    if detail.is_empty() {
                        bail!("请确认 Tailscale 已启动并登录");
                    }
                    bail!("请确认 Tailscale 已启动并登录：{detail}");
                }
                let ip: std::net::Ipv4Addr = String::from_utf8(output.stdout)?.trim().parse()?;
                Ok(ip.to_string())
            })
            .await;
            let _ = sender.send(Event::TailscaleAddress(result)).await;
        })
        .detach();
    }
    fn refresh(&mut self) {
        self.refresh_agent_activity();
        self.native_images
            .retain(|_, target| self.session.paste_target_current(target));
        // Explicit UI actions revalidate links, including files created or deleted
        // without new terminal output. Only output-triggered refreshes may reuse a frame.
        self.refresh_snapshot(true);
    }
    fn refresh_snapshot(&mut self, force: bool) -> bool {
        if let (Some(t), Some(w)) = (
            self.session
                .selected()
                .and_then(|id| self.session.terminals.get(&id)),
            self.session.data.workspace(),
        ) {
            let frame = {
                if let Ok(b) = t.buffer.lock() {
                    self.snapshot_cache
                        .capture(t.instance, &b, &w.directory, force)
                } else {
                    self.snapshot = Arc::default();
                    return self.snapshot_cache.clear();
                }
            };
            if let Some(frame) = frame {
                // Filesystem probes happen after the PTY buffer guard has dropped.
                self.snapshot = Arc::new(frame.resolve_links(&w.directory));
                return true;
            }
            return false;
        }
        self.snapshot = Arc::default();
        self.snapshot_cache.clear()
    }
    fn install_preview(
        &mut self,
        preview: PreviewContent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preview_editor = None;
        self.preview_subscription = None;
        self.preview_dirty = false;
        self.preview_editing = false;
        if let PreviewContent::Code(code) = &preview {
            let language = code
                .file
                .path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("text")
                .to_owned();
            let value = code.file.content.clone();
            let input = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language(language)
                    .default_value(value)
                    .soft_wrap(self.preview_wrap)
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
            input.update(cx, |state, cx| {
                state.set_cursor_position(Position::new(line, column), window, cx)
            });
            let subscription = cx.subscribe_in(&input, window, |ui, state, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    ui.preview_dirty = match &ui.preview {
                        Some(PreviewContent::Code(code)) => {
                            state.read(cx).value().replace("\r\n", "\n")
                                != code.file.content.replace("\r\n", "\n")
                        }
                        _ => false,
                    };
                    cx.notify();
                }
            });
            self.preview_editor = Some(input);
            self.preview_subscription = Some(subscription);
        }
        self.preview = Some(preview);
    }
    fn save_preview(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.preview_editing || !self.preview_dirty {
            return true;
        }
        let Some(input) = &self.preview_editor else {
            return true;
        };
        let text = input.read(cx).value().to_string();
        if let Some(PreviewContent::Code(code)) = &mut self.preview {
            match code.file.save(&text) {
                Ok(()) => {
                    self.preview_dirty = false;
                    self.session.error = None;
                    cx.notify();
                    true
                }
                Err(e) => {
                    self.session.error = Some(format!("保存失败：{e:#}"));
                    cx.notify();
                    false
                }
            }
        } else {
            true
        }
    }
    fn clear_preview(&mut self) {
        self.browser.borrow_mut().hide();
        self.browser = Rc::new(RefCell::new(crate::browser::Browser::new()));
        self.preview = None;
        self.split_preview_owner = None;
        self.preview_request = None;
        self.preview_editor = None;
        self.preview_subscription = None;
        self.preview_dirty = false;
        self.preview_editing = false;
    }
    fn request_preview_action(
        &mut self,
        action: PreviewAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preview_dirty {
            self.pending_document = None;
            self.pending_preview = Some(action);
            self.open_dialog(Dialog::UnsavedPreview, window, cx);
        } else {
            self.perform_preview_action(action, window, cx);
        }
    }
    fn perform_preview_action(
        &mut self,
        action: PreviewAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            PreviewAction::Close => {
                self.clear_preview();
                self.focus_current_view(window, cx);
            }
            PreviewAction::CloseDocument(id) => self.close_document(id, window, cx),
            PreviewAction::File(link) => {
                self.clear_preview();
                self.split_preview_owner = self.session.selected();
                self.open_file(link);
            }
            PreviewAction::Web(url) => {
                self.panel.open = true;
                self.panel.tab = PanelTab::Explorer;
                self.clear_preview();
                if vibe_terminal::paths::is_web_url(&url) {
                    self.preview = Some(PreviewContent::Web(WebPreview {
                        id: Uuid::new_v4(),
                        url,
                        error: None,
                    }));
                }
            }
            PreviewAction::Workspace(id) => {
                self.clear_preview();
                self.select_workspace(id);
                self.focus_current_view(window, cx);
            }
            PreviewAction::AddRemoteWorkspace {
                host_id,
                path,
                name,
            } => {
                self.clear_preview();
                match self.session.add_remote_workspace(host_id, path, name) {
                    Ok(id) => {
                        if !self.expanded_workspaces.contains(&id) {
                            self.expanded_workspaces.push(id);
                        }
                    }
                    Err(error) => self.session.error = Some(error.to_string()),
                }
                self.settings = false;
                self.active_document = None;
                self.refresh();
                self.focus.focus(window, cx);
            }
            PreviewAction::AddWorkspace(path) => {
                self.active_document = None;
                self.clear_preview();
                self.session.add_workspace(&path);
                self.refresh();
            }
            PreviewAction::RemoveWorkspace(id) => {
                if self.guard_dirty_documents(
                    Some(id),
                    PreviewAction::RemoveWorkspace(id),
                    window,
                    cx,
                ) {
                    return;
                }
                self.documents.retain(|tab| tab.workspace != id);
                if self
                    .active_document
                    .is_some_and(|id| !self.documents.iter().any(|tab| tab.id == id))
                {
                    self.active_document = None;
                }
                self.clear_preview();
                self.session.remove_workspace(id);
                self.refresh();
            }
            PreviewAction::Quit => {
                if self.confirm_window_close(window, cx) {
                    cx.quit();
                }
            }
        }
        cx.notify();
    }
    fn finish_pending_preview(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.pending_document {
            if save && !self.save_document(id, cx) {
                return;
            }
            if let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == id) {
                tab.dirty = false;
            }
            self.pending_document = None;
        } else {
            if save && !self.save_preview(cx) {
                return;
            }
            self.preview_dirty = false;
        }
        let action = self.pending_preview.take();
        self.close_dialog(window, cx);
        if let Some(action) = action {
            self.perform_preview_action(action, window, cx);
        }
    }
    fn request_workspace(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.request_preview_action(PreviewAction::Workspace(id), window, cx);
    }
    pub fn confirm_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.preview_dirty {
            self.request_preview_action(PreviewAction::Quit, window, cx);
            false
        } else {
            !self.guard_dirty_documents(None, PreviewAction::Quit, window, cx)
        }
    }
    fn select_workspace(&mut self, id: Uuid) {
        self.active_document = None;
        self.clear_mouse_hover();
        self.settings = false;
        self.editor = None;
        self.wheel.reset();
        self.session.select_workspace(id);
        self.preview = None;
        self.preview_request = None;
        self.selection = None;
        self.preedit.clear();
        self.refresh();
    }
    fn select_tab(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.active_document = None;
        self.clear_mouse_hover();
        self.wheel.reset();
        self.session.select_tab(id);
        self.selection = None;
        self.preedit.clear();
        // 首次打开休眠标签才恢复；已打开（包括已退出）的运行时驻留到关闭。
        if !self
            .session
            .terminals
            .get(&id)
            .is_some_and(|terminal| terminal.loaded())
        {
            self.session.restore(id);
        }
        self.refresh();
        self.focus_current_view(window, cx);
    }
    fn toggle_workspace_expanded(&mut self, id: Uuid) {
        if let Some(index) = self.expanded_workspaces.iter().position(|item| *item == id) {
            self.expanded_workspaces.remove(index);
        } else {
            self.expanded_workspaces.push(id);
        }
    }
    fn open_tab(
        &mut self,
        workspace: Uuid,
        tab: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session.data.selected_workspace != Some(workspace) {
            self.request_workspace(workspace, window, cx);
        }
        if self.session.data.selected_workspace == Some(workspace) {
            self.select_tab(tab, window, cx);
        }
    }
    fn button_style(selected: bool, cx: &App) -> ButtonCustomVariant {
        ButtonCustomVariant::new(cx)
            .color(if selected {
                rgb(0x2c4035).into()
            } else {
                rgba(0).into()
            })
            .foreground(rgb(if selected { ACCENT } else { FG }).into())
            .hover(rgb(if selected { 0x486451 } else { 0x414c45 }).into())
            .active(rgb(0x587360).into())
    }
    fn tool_button(
        id: impl Into<ElementId>,
        label: &'static str,
        icon: gpui_kit::assets::IconName,
        selected: bool,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        Button::new(id)
            .with_size(gpui_kit::component::Size::Size(px(20.0)))
            .size(px(20.0))
            .p_0()
            .child(control_icon(icon))
            .custom(Self::button_style(selected, cx))
            .accessibility_label(label)
            .tooltip(label)
            .on_click(cx.listener(move |s, _, w, cx| {
                cx.stop_propagation();
                action(s, w, cx);
                cx.notify();
            }))
    }
    fn button(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
        f: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        Button::new(id)
            .label(label)
            .small()
            .custom(Self::button_style(false, cx))
            .on_click(cx.listener(move |ui, _, window, cx| {
                f(ui, window, cx);
                cx.notify();
            }))
    }
    fn primary(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
        f: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        Button::new(id)
            .label(label)
            .small()
            .primary()
            .on_click(cx.listener(move |ui, _, window, cx| {
                f(ui, window, cx);
                cx.notify();
            }))
    }
    fn new_terminal_with(
        &mut self,
        profile: CliProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_document = None;
        self.settings = false;
        self.editor = None;
        self.wheel.reset();
        self.session.new_tab(profile);
        self.selection = None;
        self.new_menu = None;
        self.refresh();
        self.focus.focus(window, cx);
    }
    fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let profile = if self
            .session
            .data
            .workspace()
            .is_some_and(|w| w.remote.is_some())
        {
            vibe_terminal::ssh::profiles().remove(0)
        } else {
            CliProfile::default_shell()
        };
        self.new_terminal_with(profile, window, cx);
    }
    fn new_terminal_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some((x, y)) = self.new_menu else {
            return div().into_any_element();
        };
        let mut panel = div()
            .id("new-terminal-menu")
            .test_support()
            .max_h(px(480.0))
            .overflow_y_scroll()
            .absolute()
            .left(px(x))
            .top(px(y.max(40.0) + 6.0))
            .w(px(232.0))
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(rgb(0x454545))
            .bg(rgb(SURFACE))
            .child(
                div()
                    .px_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("新建终端"),
            );
        let profiles = self.menu_profiles();
        if profiles.is_empty() {
            panel = panel.child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("没有可用的 CLI"),
            );
        }
        for profile in profiles {
            let name = profile.name.clone();
            let glyph = profile_icon(&profile);
            let category = if profile.kind == CliKind::Shell {
                "Shell"
            } else {
                "Agent"
            };
            panel = panel.child(
                div()
                    .id(SharedString::from(format!("new-terminal-{}", profile.id)))
                    .test_support()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .h(px(36.0))
                    .rounded_md()
                    .hover(|style| style.bg(rgb(0x313131)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |s, _, window, cx| {
                        s.new_terminal_with(profile.clone(), window, cx);
                        cx.notify();
                    }))
                    .child(glyph)
                    .child(div().flex_1().text_sm().child(name))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(category)),
            );
        }
        div()
            .absolute()
            .inset_0()
            .size_full()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|s, _, _, cx| {
                    s.new_menu = None;
                    cx.notify();
                }),
            )
            .child(panel.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()))
            .into_any_element()
    }
    /// ＋ 菜单内容：内置 CMD / PowerShell 在前，其后是检测到且已启用的 Agent。
    fn menu_profiles(&self) -> Vec<CliProfile> {
        if self
            .session
            .data
            .workspace()
            .is_some_and(|w| w.remote.is_some())
        {
            return vibe_terminal::ssh::profiles();
        }
        let mut profiles = self.session.data.profiles.clone();
        for detected in &self.detected_agents {
            if self
                .session
                .data
                .disabled_agents
                .iter()
                .any(|id| id == detected.spec.id)
            {
                continue;
            }
            profiles.push(vibe_terminal::agents::profile(detected));
        }
        profiles
    }
    fn open_settings(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.detected_agents = vibe_terminal::agents::detected();
        self.editor = None;
        self.dialog = None;
        self.settings = true;
        cx.notify();
    }
    fn web_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut section = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("开启后，同一局域网或 Tailscale 网络中的设备可以在浏览器里查看并操作终端。开启时会把带令牌的局域网链接复制到剪贴板。"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(Self::button(
                        "toggle-web",
                        if self.web_server.is_some() {
                            "关闭 Web 访问"
                        } else {
                            "开启 Web 访问"
                        },
                        cx,
                        |s, _, cx| {
                            s.toggle_web(cx);
                            if s.web_server.is_some() {
                                s.find_tailscale(cx);
                            }
                        },
                    ))
                    .child(
                        div().text_xs().text_color(rgb(MUTED)).child(match &self.web_server {
                            Some(server) => format!("已开启 · 端口 {}", server.address.port()),
                            None => "未开启".to_owned(),
                        }),
                    ),
            );
        if let Some(server) = self.web_server.as_ref() {
            section = section.child(Self::web_link_row(
                "局域网链接",
                server.lan_url(),
                "copy-web-lan",
                cx,
            ));
            match self.web_tailscale.as_ref() {
                Some(ip) => {
                    section = section.child(Self::web_link_row(
                        "Tailscale 链接",
                        server.url(ip),
                        "copy-web-tailscale",
                        cx,
                    ));
                }
                None => {
                    section = section.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().w(px(120.0)).text_sm().child("Tailscale 链接"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child("未检测到 Tailscale 地址"),
                            )
                            .child(Self::button(
                                "refresh-tailscale",
                                "重新获取",
                                cx,
                                |s, _, cx| {
                                    s.find_tailscale(cx);
                                },
                            )),
                    );
                }
            }
            section = section.child(Self::web_link_row(
                "访问令牌",
                server.token.clone(),
                "copy-web-token",
                cx,
            ));
        }
        section.into_any_element()
    }
    fn web_link_row(
        label: &'static str,
        value: String,
        id: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let copy = value.clone();
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(div().w(px(120.0)).text_sm().child(label))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(value),
            )
            .child(Self::button(id, "复制", cx, move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
            }))
            .into_any_element()
    }
    fn settings_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut nav = div()
            .flex()
            .flex_col()
            .gap_1()
            .w(px(SIDEBAR_WIDTH))
            .flex_shrink_0()
            .h_full()
            .px_2()
            .pb_2()
            .bg(rgb(PANEL))
            .border_r_1()
            .border_color(rgb(0x303030))
            .child(
                div()
                    .h(px(48.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .px_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("设置"),
            );
        for tab in [
            SettingsTab::General,
            SettingsTab::Agents,
            SettingsTab::Shortcuts,
            SettingsTab::SshHosts,
            SettingsTab::Remote,
        ] {
            let active = self.settings_tab == tab;
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("settings-nav-{}", tab.title())))
                    .w_full()
                    .flex()
                    .items_center()
                    .px_2()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .bg(rgb(if active { SURFACE } else { PANEL }))
                    .text_color(rgb(if active { ACCENT } else { FG }))
                    .on_click(cx.listener(move |s, _, window, cx| {
                        s.select_settings_tab(tab, window, cx);
                        cx.notify();
                    }))
                    .child(div().text_sm().child(tab.title())),
            );
        }
        let nav = nav.child(div().flex_1()).child(self.navigation_footer(cx));
        let body = match self.settings_tab {
            SettingsTab::General => self.general_pane(cx),
            SettingsTab::Agents => self.agents_pane(cx),
            SettingsTab::Shortcuts => self.shortcuts_pane(),
            SettingsTab::Remote => self.web_settings(cx),
            SettingsTab::SshHosts => self.ssh_settings(cx),
        };
        div()
            .id("settings-page")
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(nav)
            .child(
                div()
                    .id("settings-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_6()
                    .gap_4()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.settings_tab.title()),
                    )
                    .child(body),
            )
            .into_any_element()
    }
    fn general_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_sm().child("输入光标样式"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("选择终端输入位置的光标形状。默认小方块，修改后立即生效并自动保存。"),
            )
            .child(
                div().flex().gap_2().children(
                    [
                        ("cursor-style-block", "小方块", InputCursorStyle::Block),
                        ("cursor-style-bar", "竖线", InputCursorStyle::Bar),
                    ]
                    .into_iter()
                    .map(|(id, label, style)| {
                        Button::new(id)
                            .small()
                            .label(label)
                            .selected(self.session.data.input_cursor_style == style)
                            .on_click(cx.listener(move |ui, _, _, cx| {
                                if ui.session.data.input_cursor_style != style {
                                    ui.session.data.input_cursor_style = style;
                                    ui.session.persist();
                                    cx.notify();
                                }
                            }))
                    }),
                ),
            )
            .into_any_element()
    }
    fn agents_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut pane = div().flex().flex_col().gap_3().child(
            div()
                .text_sm()
                .text_color(rgb(MUTED))
                .child("自动扫描 PATH 与常见安装目录，只列出本机已安装的 Agent；CMD 与 PowerShell 是内置终端，固定排在 ＋ 菜单最前，不在此列。"),
        );
        if self.detected_agents.is_empty() {
            pane = pane.child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(PANEL))
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("未在本机检测到已安装的 Agent。安装后重新打开设置即可刷新。"),
            );
        }
        for detected in &self.detected_agents {
            let id = detected.spec.id;
            let enabled = !self
                .session
                .data
                .disabled_agents
                .iter()
                .any(|item| item == id);
            pane = pane.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(PANEL))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().child(detected.spec.name))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child(detected.path.display().to_string()),
                            ),
                    )
                    .child(Self::button(
                        SharedString::from(format!("agent-toggle-{id}")),
                        if enabled { "已启用" } else { "已隐藏" },
                        cx,
                        move |s, _, _| s.toggle_agent(id),
                    )),
            );
        }
        pane.into_any_element()
    }
    fn select_settings_tab(
        &mut self,
        tab: SettingsTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_tab = tab;
        if tab == SettingsTab::Agents {
            self.detected_agents = vibe_terminal::agents::detected();
        }
        self.focus.focus(window, cx);
    }
    fn shortcuts_pane(&self) -> AnyElement {
        const SHORTCUTS: &[(&str, &str)] = &[
            ("Ctrl + T", "新建终端（默认 CMD）"),
            ("Ctrl + W", "关闭当前标签"),
            (
                "Ctrl + Tab / Ctrl + Shift + Tab",
                "切换到下一个 / 上一个标签",
            ),
            ("Ctrl + 1 … 9", "跳到第 N 个标签"),
            ("Ctrl + ,", "打开设置"),
            ("Ctrl + Shift + C", "复制选中的输出"),
            ("Ctrl + V", "粘贴到终端"),
            ("滚轮 / Shift + 滚轮", "滚动历史；Shift 只滚动本地历史"),
            ("Ctrl + 左键", "打开文件或图片预览（macOS 用 ⌘）"),
            ("Ctrl + / 或 F1", "打开设置里的快捷键页"),
            ("Esc", "关闭对话框、菜单或快捷键页"),
        ];
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(SHORTCUTS.iter().map(|(keys, description)| {
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .py_1()
                    .child(
                        div()
                            .w(px(260.0))
                            .text_xs()
                            .text_color(rgb(ACCENT))
                            .child(*keys),
                    )
                    .child(div().flex_1().text_sm().child(*description))
            }))
            .into_any_element()
    }
    fn toggle_agent(&mut self, id: &str) {
        if let Some(index) = self
            .session
            .data
            .disabled_agents
            .iter()
            .position(|item| item == id)
        {
            self.session.data.disabled_agents.remove(index);
        } else {
            self.session.data.disabled_agents.push(id.to_owned());
        }
        self.session.persist();
    }
    fn choose_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_workspace_dialog(window, cx);
    }
    fn pick_local_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择本地项目目录".into()),
        });
        cx.spawn_in(window, async move |weak, cx| match paths.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.first() {
                    let _ = weak.update_in(cx, |ui, window, cx| {
                        ui.request_preview_action(
                            PreviewAction::AddWorkspace(path.clone()),
                            window,
                            cx,
                        );
                    });
                }
            }
            Ok(Err(e)) => {
                let _ = weak.update(cx, |ui, cx| {
                    ui.session.error = Some(format!("无法选择目录：{e}"));
                    cx.notify();
                });
            }
            _ => {}
        })
        .detach();
    }
    fn load_image(&mut self, path: PathBuf) {
        let id = Uuid::new_v4();
        self.preview_request = Some(id);
        self.preview = None;
        let tx = self.sender.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<PreviewContent> {
                if std::fs::metadata(&path)?.len() > 16 * 1024 * 1024 {
                    bail!("图片超过 16 MB，请选择较小的图片");
                }
                let (w, h) = image::ImageReader::open(&path)?
                    .with_guessed_format()?
                    .into_dimensions()?;
                if w as u64 * h as u64 > 16_000_000 {
                    bail!("图片超过 1600 万像素，请先缩小图片");
                }
                Ok(PreviewContent::Image(path))
            })();
            let _ = tx.send_blocking(Event::Preview(id, result));
        });
    }
    fn open_file(&mut self, link: FileLink) {
        let extension = link
            .path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(
            extension.as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
        ) {
            self.load_image(link.path);
            return;
        }
        let id = Uuid::new_v4();
        self.preview_request = Some(id);
        self.preview = None;
        let tx = self.sender.clone();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(Event::Preview(
                id,
                load_code_preview(link).map(PreviewContent::Code),
            ));
        });
    }
    fn open_dialog(&mut self, dialog: Dialog, window: &mut Window, cx: &mut Context<Self>) {
        self.settings = false;
        let value = match &dialog {
            Dialog::RenameWorkspace(id) => self
                .session
                .data
                .workspaces
                .iter()
                .find(|w| w.id == *id)
                .map(|w| w.name.clone())
                .unwrap_or_default(),
            Dialog::RenameTab(id) => self
                .session
                .data
                .tab(*id)
                .map(|t| t.display_title().to_owned())
                .unwrap_or_default(),
            _ => String::new(),
        };
        self.editor = match dialog {
            Dialog::RenameWorkspace(_) | Dialog::RenameTab(_) => {
                let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
                input.update(cx, |s, cx| s.focus(window, cx));
                Some(Editor::Single(input))
            }
            _ => None,
        };
        self.dialog = Some(dialog);
    }
    fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let restore_file = matches!(self.dialog, Some(Dialog::UnsavedPreview));
        self.dialog = None;
        self.editor = None;
        self.pending_preview = None;
        self.pending_document = None;
        if restore_file
            && self.active_document.is_none()
            && let Some(input) = &self.preview_editor
        {
            input.update(cx, |state, cx| state.focus(window, cx));
        } else {
            self.focus_current_view(window, cx);
        }
    }
    fn apply_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        let value = match &self.editor {
            Some(Editor::Single(s)) => s.read(cx).value().to_string(),
            None => String::new(),
        };
        let value = value.trim().to_owned();
        match dialog {
            Dialog::RenameWorkspace(id) => {
                if value.is_empty() {
                    self.session.error = Some("名称不能为空".into());
                    return;
                }
                if let Some(w) = self.session.data.workspaces.iter_mut().find(|w| w.id == id) {
                    w.name = value;
                }
            }
            Dialog::RenameTab(id) => {
                if value.is_empty() {
                    self.session.error = Some("名称不能为空".into());
                    return;
                }
                if let Some(t) = self.session.data.tab_mut(id) {
                    t.rename(value);
                }
            }
            Dialog::RemoveWorkspace(id) => {
                self.close_dialog(window, cx);
                self.request_preview_action(PreviewAction::RemoveWorkspace(id), window, cx);
                return;
            }
            Dialog::CloseTab(id) => self.session.close_tab(id),
            Dialog::UnsavedPreview => {
                self.finish_pending_preview(false, window, cx);
                return;
            }
        }
        self.session.error = None;
        self.session.persist();
        self.refresh();
        self.close_dialog(window, cx);
    }
    fn copy_selection(&self, cx: &mut App) {
        if let Some((a, b)) = self.selection {
            cx.write_to_clipboard(ClipboardItem::new_string(self.snapshot.selected_text(a, b)));
        }
    }
    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if self.ssh.modal.is_some() {
            if key == "escape" {
                self.ssh.modal = None;
                self.focus_current_view(window, cx);
                cx.notify();
                cx.stop_propagation();
            }
            return;
        }
        let m = event.keystroke.modifiers;
        let primary = if cfg!(target_os = "macos") {
            m.platform
        } else {
            m.control
        };
        if key == "f1" || (primary && key == "/") {
            self.settings_tab = SettingsTab::Shortcuts;
            self.editor = None;
            self.dialog = None;
            self.settings = true;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.new_menu.is_some() && key == "escape" {
            self.new_menu = None;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.dialog.is_some() {
            if key == "escape" {
                self.close_dialog(window, cx);
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }
        if self.content_visible() && self.focus.is_focused(window) && key == "escape" {
            self.toggle_transcript(window, cx);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.settings && key == "escape" {
            self.settings = false;
            self.editor = None;
            self.focus_current_view(window, cx);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let file_focused = self
            .preview_editor
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window));
        if primary && key == "s" && file_focused {
            self.save_preview(cx);
            cx.stop_propagation();
            return;
        }
        if primary && key == "w" && file_focused {
            self.request_preview_action(PreviewAction::Close, window, cx);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if primary && key == "s" && self.active_document.is_some() {
            self.save_document(self.active_document.unwrap(), cx);
            cx.stop_propagation();
            return;
        }
        if primary && key == "t" {
            self.new_terminal(window, cx);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if primary && key == "," {
            self.open_settings(window, cx);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if primary && key == "w" {
            if self.settings {
                self.settings = false;
                self.editor = None;
                self.focus_current_view(window, cx);
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if let Some(id) = self.active_document {
                self.request_close_document(id, window, cx);
            } else if let Some(id) = self.session.selected() {
                self.open_dialog(Dialog::CloseTab(id), window, cx);
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if !self.settings && !file_focused && primary {
            if let Some(index) = key
                .chars()
                .next()
                .and_then(|c| c.to_digit(10))
                .filter(|digit| (1..=9).contains(digit))
            {
                if let Some(target) = self.main_tabs().get(index as usize - 1).copied() {
                    self.select_main_tab(target, window, cx);
                }
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }
        if m.control && key == "tab" {
            let tabs = self.main_tabs();
            if !tabs.is_empty() {
                let index = tabs
                    .iter()
                    .position(|target| Some(*target) == self.current_main_tab())
                    .unwrap_or(0);
                let next = if m.shift {
                    (index + tabs.len() - 1) % tabs.len()
                } else {
                    (index + 1) % tabs.len()
                };
                self.select_main_tab(tabs[next], window, cx);
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let image_paste_key = (primary && key == "v")
            || (m.alt && !m.control && key == "v")
            || (m.shift && key == "insert");
        if image_paste_key && self.try_paste_image(window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.active_document.is_some()
            || self.content_visible()
            || !self.focus.is_focused(window)
        {
            return;
        }
        if primary
            && key == "c"
            && (cfg!(target_os = "macos") || m.shift || self.selection.is_some_and(|(a, b)| a != b))
        {
            self.copy_selection(cx);
            cx.stop_propagation();
            return;
        }
        if primary && key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                self.session.paste(&text);
            }
            cx.stop_propagation();
            return;
        }
        let cursor = self
            .session
            .selected()
            .and_then(|id| self.session.terminals.get(&id))
            .and_then(|t| {
                t.buffer
                    .lock()
                    .ok()
                    .map(|b| b.parser.screen().application_cursor())
            })
            .unwrap_or(false);
        if let Some(bytes) = key_bytes(key, m.control, m.alt, m.shift, cursor) {
            let has_image = self
                .session
                .selected()
                .and_then(|id| self.native_images.get(&id))
                .is_some_and(|target| self.session.paste_target_current(target));
            self.session.send_input(bytes, has_image);
            self.selection = None;
            cx.stop_propagation();
            cx.notify();
        }
    }
    fn forward_mouse(
        &mut self,
        id: Uuid,
        point: CellPoint,
        event: vibe_terminal::scroll::MouseButtonEvent,
        modifiers: Modifiers,
    ) -> bool {
        let fullscreen_cli = self.session.data.tab(id).is_some_and(|tab| {
            matches!(
                tab.profile.kind,
                CliKind::Claude | CliKind::Codex | CliKind::Pi
            )
        });
        let sequence = self
            .session
            .terminals
            .get(&id)
            .filter(|t| t.running())
            .and_then(|t| {
                t.buffer.lock().ok().and_then(|buffer| {
                    let screen = buffer.parser.screen();
                    let (rows, cols) = screen.size();
                    // Hover leave uses an out-of-grid coordinate so the TUI
                    // can clear its active scrollbar even beyond this pane.
                    let extra = u16::from(matches!(
                        event,
                        vibe_terminal::scroll::MouseButtonEvent::Move
                    ));
                    vibe_terminal::scroll::mouse_button_sequence(
                        screen,
                        event,
                        (point.0 as u16).min(rows.saturating_sub(1) + extra),
                        (point.1 as u16).min(cols.saturating_sub(1) + extra),
                        modifiers.alt,
                        modifiers.control,
                        fullscreen_cli,
                    )
                })
            });
        let Some(sequence) = sequence else {
            return false;
        };
        if let Some(terminal) = self.session.terminals.get(&id) {
            let result = terminal.send(sequence);
            self.session.result(result);
        }
        true
    }
    fn clear_mouse_hover(&mut self) {
        if let Some((id, _)) = self.mouse_hover.take() {
            self.forward_mouse(
                id,
                (u16::MAX as usize, u16::MAX as usize),
                vibe_terminal::scroll::MouseButtonEvent::Move,
                Modifiers::default(),
            );
        }
    }
    fn terminal_cursor(&self) -> CursorStyle {
        if let Some(position) = self.terminal_pointer {
            let x = f32::from(position.x - self.bounds.origin.x) - INSET;
            let y = f32::from(position.y - self.bounds.origin.y) - INSET;
            if x >= 0.0
                && y >= 0.0
                && x < self.snapshot.cols as f32 * CELL
                && y < self.snapshot.rows as f32 * LINE
            {
                let row = (y / LINE).floor() as usize;
                let col = (x / CELL).floor() as usize;
                if self.snapshot.link_at(row, col).is_some()
                    || self.snapshot.web_link_at(row, col).is_some()
                {
                    return CursorStyle::PointingHand;
                }
            }
        }
        CursorStyle::Arrow
    }
    fn terminal_hover(&mut self, hovered: &bool, _window: &mut Window, cx: &mut Context<Self>) {
        if !hovered {
            if self.terminal_pointer.take().is_some() {
                cx.notify();
            }
            if self.mouse_capture.is_none() {
                self.clear_mouse_hover();
            }
        }
    }
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let point = render::cell_at(
            event.position,
            self.bounds,
            self.snapshot.rows,
            self.snapshot.cols,
        );
        let primary = if cfg!(target_os = "macos") {
            event.modifiers.platform
        } else {
            event.modifiers.control
        };
        if primary {
            self.path_press = Some((point, event.position));
            self.selecting = false;
        } else if !event.modifiers.shift
            && let Some(id) = self.session.selected()
            && self.forward_mouse(
                id,
                point,
                vibe_terminal::scroll::MouseButtonEvent::Press,
                event.modifiers,
            )
        {
            self.mouse_capture = Some((id, point));
            self.selection = None;
            self.selecting = false;
            self.path_press = None;
            cx.stop_propagation();
        } else {
            self.selection = Some((point, point));
            self.selecting = true;
            self.path_press = None;
        }
        cx.notify();
    }
    fn mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let previous_cursor = self.terminal_cursor();
        self.terminal_pointer = Some(event.position);
        if previous_cursor != self.terminal_cursor() {
            cx.notify();
        }
        if let Some((origin, width)) = self.resize_origin {
            if self.resizing_terminal_preview {
                self.terminal_preview_width =
                    (width - f32::from(event.position.x - origin)).clamp(120.0, 800.0);
            } else {
                self.preview_width =
                    (width - f32::from(event.position.x - origin)).clamp(280.0, 800.0);
            }
            cx.notify();
            return;
        }
        if let Some((id, previous)) = self.mouse_capture {
            let point = render::cell_at(
                event.position,
                self.bounds,
                self.snapshot.rows,
                self.snapshot.cols,
            );
            if !event.dragging() {
                self.forward_mouse(
                    id,
                    point,
                    vibe_terminal::scroll::MouseButtonEvent::Release,
                    event.modifiers,
                );
                self.mouse_capture = None;
                self.mouse_hover = Some((id, point));
                if !self.bounds.contains(&event.position) {
                    self.clear_mouse_hover();
                }
            } else if point != previous {
                self.forward_mouse(
                    id,
                    point,
                    vibe_terminal::scroll::MouseButtonEvent::Drag,
                    event.modifiers,
                );
                self.mouse_capture = Some((id, point));
            }
            cx.stop_propagation();
            return;
        }
        if !self.selecting && event.pressed_button.is_none() {
            if !self.settings
                && self.dialog.is_none()
                && self.new_menu.is_none()
                && !self.content_visible()
                && self.bounds.contains(&event.position)
                && let Some(id) = self.session.selected()
            {
                let (row, col) = render::cell_at(
                    event.position,
                    self.bounds,
                    self.snapshot.rows,
                    self.snapshot.cols,
                );
                let point = (row, col.min(self.snapshot.cols.saturating_sub(1) as usize));
                if self.mouse_hover != Some((id, point)) {
                    if self.mouse_hover.is_some_and(|(previous, _)| previous != id) {
                        self.clear_mouse_hover();
                    }
                    if self.forward_mouse(
                        id,
                        point,
                        vibe_terminal::scroll::MouseButtonEvent::Move,
                        event.modifiers,
                    ) {
                        self.mouse_hover = Some((id, point));
                    }
                }
                cx.stop_propagation();
            } else {
                self.clear_mouse_hover();
            }
        }
        if self.selecting {
            if let Some((start, _)) = self.selection {
                self.selection = Some((
                    start,
                    render::cell_at(
                        event.position,
                        self.bounds,
                        self.snapshot.rows,
                        self.snapshot.cols,
                    ),
                ));
                cx.notify();
            }
        }
    }
    fn mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        self.resize_origin = None;
        if let Some((id, _)) = self.mouse_capture.take() {
            let point = render::cell_at(
                event.position,
                self.bounds,
                self.snapshot.rows,
                self.snapshot.cols,
            );
            self.forward_mouse(
                id,
                point,
                vibe_terminal::scroll::MouseButtonEvent::Release,
                event.modifiers,
            );
            self.mouse_hover = Some((id, point));
            if !self.bounds.contains(&event.position) {
                self.clear_mouse_hover();
            }
            cx.stop_propagation();
            return;
        }
        if let Some((point, start)) = self.path_press.take() {
            if (f32::from(event.position.x - start.x)).abs() < 4.0
                && (f32::from(event.position.y - start.y)).abs() < 4.0
            {
                if let Some(link) = self.snapshot.link_at(point.0, point.1).cloned() {
                    self.request_preview_action(PreviewAction::File(link), window, cx);
                    cx.notify();
                } else if let Some(link) = self.snapshot.web_link_at(point.0, point.1) {
                    self.request_preview_action(PreviewAction::Web(link.url.clone()), window, cx);
                    cx.notify();
                }
            }
        }
    }
    fn wheel(&mut self, event: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let delta = f32::from(event.delta.pixel_delta(px(LINE)).y);
        let lines = self.wheel.lines(delta, LINE);
        if lines == 0 {
            cx.stop_propagation();
            return;
        }
        let position = render::cell_at(
            event.position,
            self.bounds,
            self.snapshot.rows,
            self.snapshot.cols,
        );
        // Windows 的 ConPTY 会吞掉全屏 CLI 的鼠标模式序列，此时按标签类型强制转发滚轮。
        let fullscreen_cli = self
            .session
            .selected()
            .and_then(|id| self.session.data.tab(id))
            .is_some_and(|tab| {
                matches!(
                    tab.profile.kind,
                    CliKind::Claude | CliKind::Codex | CliKind::Pi
                )
            });
        let mouse = self
            .session
            .selected()
            .and_then(|id| self.session.terminals.get(&id))
            .filter(|t| t.running())
            .and_then(|t| {
                t.buffer.lock().ok().and_then(|buffer| {
                    vibe_terminal::scroll::mouse_wheel_sequence(
                        buffer.parser.screen(),
                        lines > 0,
                        position.0 as u16,
                        position.1 as u16,
                        event.modifiers.alt,
                        event.modifiers.control,
                        fullscreen_cli,
                    )
                })
            });
        if !event.modifiers.shift
            && let Some(sequence) = mouse
        {
            self.session
                .send(sequence.repeat(lines.unsigned_abs().min(100)));
            cx.stop_propagation();
            return;
        }
        if let Some(t) = self
            .session
            .selected()
            .and_then(|id| self.session.terminals.get(&id))
        {
            if self.snapshot.alternate && t.running() && !event.modifiers.shift {
                let count = lines.unsigned_abs().min(100);
                let sequence = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
                let result = t.send(sequence.repeat(count));
                self.session.result(result);
            } else if let Ok(mut b) = t.buffer.lock() {
                b.scroll_lines(lines);
            }
        }
        self.selection = None;
        self.refresh();
        cx.stop_propagation();
        cx.notify();
    }
    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().flex().flex_col().gap_1();
        for workspace in &self.session.data.workspaces {
            let id = workspace.id;
            let active = self.session.data.selected_workspace == Some(id);
            let expanded = self.expanded_workspaces.contains(&id);
            let heading = div()
                .id(SharedString::from(format!("workspace-heading-{id}")))
                .rounded_sm()
                .cursor_pointer()
                .hover(|style| style.bg(rgb(SURFACE)))
                .on_click(cx.listener(move |s, _, window, cx| {
                    if s.session.data.selected_workspace != Some(id) {
                        s.request_workspace(id, window, cx);
                    }
                    s.toggle_workspace_expanded(id);
                    cx.notify();
                }))
                .flex()
                .items_center()
                .h(px(32.0))
                .gap_1()
                .px_1()
                .child(Self::tool_button(
                    SharedString::from(format!("workspace-toggle-{id}")),
                    "展开或折叠项目",
                    if expanded {
                        gpui_kit::assets::IconName::ChevronDown
                    } else {
                        gpui_kit::assets::IconName::ChevronRight
                    },
                    false,
                    cx,
                    move |s, _, cx| {
                        s.toggle_workspace_expanded(id);
                        cx.notify();
                    },
                ))
                .child(
                    div()
                        .id(SharedString::from(format!("workspace-{id}")))
                        .flex()
                        .items_center()
                        .gap_2()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .cursor_pointer()
                        .text_xs()
                        .text_color(rgb(if active { FG } else { MUTED }))
                        .child(icon(if workspace.remote.is_some() {
                            "server"
                        } else {
                            "folder"
                        }))
                        .child(div().min_w_0().truncate().child(workspace.name.clone())),
                )
                .child(Self::tool_button(
                    SharedString::from(format!("workspace-settings-{id}")),
                    "项目设置",
                    gpui_kit::assets::IconName::Ellipsis,
                    false,
                    cx,
                    move |s, w, cx| s.open_dialog(Dialog::RenameWorkspace(id), w, cx),
                ));
            let mut group = div().flex().flex_col().child(heading);
            if expanded {
                let mut sessions = div().flex().flex_col().gap(px(2.0)).pl_4().pb_1();
                for tab in &workspace.tabs {
                    let tab_id = tab.id;
                    let selected = active && workspace.selected_tab == Some(tab_id);
                    sessions = sessions.child(
                        div()
                            .id(SharedString::from(format!("workspace-tab-{tab_id}")))
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(28.0))
                            .px_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgb(if selected { 0x313131 } else { PANEL }))
                            .hover(|s| s.bg(rgb(SURFACE)))
                            .text_xs()
                            .text_color(rgb(if selected { FG } else { MUTED }))
                            .on_click(cx.listener(move |s, _, w, cx| {
                                s.open_tab(id, tab_id, w, cx);
                                cx.notify();
                            }))
                            .children(self.agent_activity_indicator(tab_id, "sidebar"))
                            .child(profile_icon_sized(&tab.profile, px(14.0)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(tab.display_title().to_owned()),
                            ),
                    );
                }
                group = group.child(sessions);
            }
            list = list.child(group);
        }
        div()
            .w(px(SIDEBAR_WIDTH))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .px_2()
            .pb_2()
            .bg(rgb(PANEL))
            .border_r_1()
            .border_color(rgb(0x303030))
            .child(
                div()
                    .h(px(48.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("项目")
                    .child(Self::tool_button(
                        "add-workspace-small",
                        "添加工作区",
                        gpui_kit::assets::IconName::Plus,
                        false,
                        cx,
                        Self::choose_workspace,
                    )),
            )
            .child(
                div()
                    .id("workspaces")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list),
            )
            .child(self.navigation_footer(cx))
            .into_any_element()
    }
    fn terminal(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.content_visible() {
            return self.conversation_view(cx);
        }
        let pane = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full();
        if self.session.selected().is_none() {
            return pane
                .items_center()
                .justify_center()
                .gap_4()
                .child("选择 CLI 创建终端")
                .child(Self::primary(
                    "empty-terminal",
                    "＋ 新终端",
                    cx,
                    |s, w, cx| s.new_terminal(w, cx),
                ))
                .into_any_element();
        }
        pane.child(
            div()
                .id("terminal")
                .relative()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .track_focus(&self.focus)
                .key_context("VibeTerminal")
                .cursor(self.terminal_cursor())
                .on_hover(cx.listener(Self::terminal_hover))
                .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
                .on_mouse_move(cx.listener(Self::mouse_move))
                .on_scroll(cx.listener(Self::wheel))
                .child(TerminalSurface { owner: cx.entity() }),
        )
        .into_any_element()
    }
    fn web_preview(&self, web: &WebPreview, cx: &mut Context<Self>) -> AnyElement {
        let header = div()
            .flex()
            .items_center()
            .h(px(40.0))
            .flex_shrink_0()
            .bg(rgb(PANEL))
            .px_3()
            .gap_2()
            .child(Self::button("web-back", "←", cx, |s, _, _| {
                s.browser_command("history.back()")
            }))
            .child(Self::button("web-forward", "→", cx, |s, _, _| {
                s.browser_command("history.forward()")
            }))
            .child(Self::button("web-reload", "刷新", cx, |s, _, cx| {
                if let Some(PreviewContent::Web(web)) = &s.preview {
                    if web.error.is_some() {
                        let url = web.url.clone();
                        s.browser = Rc::new(RefCell::new(crate::browser::Browser::new()));
                        s.preview = Some(PreviewContent::Web(WebPreview {
                            id: Uuid::new_v4(),
                            url,
                            error: None,
                        }));
                    } else {
                        s.browser_command("location.reload()");
                    }
                }
                cx.notify();
            }))
            .child(Self::button(
                "web-devtools",
                "开发者工具",
                cx,
                |s, _, _| {
                    if matches!(s.preview, Some(PreviewContent::Web(_))) {
                        s.browser.borrow().open_devtools();
                    }
                },
            ))
            .child(div().flex_1())
            .child(Self::button("close-web", "×", cx, |s, w, cx| {
                s.request_preview_action(PreviewAction::Close, w, cx)
            }));
        let address = div()
            .flex()
            .items_center()
            .px_3()
            .pb_2()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .child(web.url.clone()),
            )
            .child(Self::button(
                "copy-web-url",
                "复制地址",
                cx,
                |s, _, cx| {
                    if let Some(PreviewContent::Web(web)) = &s.preview {
                        cx.write_to_clipboard(ClipboardItem::new_string(web.url.clone()));
                    }
                },
            ))
            .child(Self::button(
                "open-web-external",
                "浏览器打开",
                cx,
                |s, _, cx| {
                    if let Some(PreviewContent::Web(web)) = &s.preview {
                        cx.open_url(&web.url);
                    }
                },
            ));
        let mut pane = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .bg(rgb(BG))
            .child(header)
            .child(address);
        if let Some(error) = &web.error {
            pane = pane.child(div().p_4().text_sm().child(format!(
                "网页预览启动失败：{error}。可点击刷新重试，或使用浏览器打开。"
            )));
        } else if self.dialog.is_none() && self.ssh.modal.is_none() && self.new_menu.is_none() {
            let browser = self.browser.clone();
            let url = web.url.clone();
            let data = self.session.store.root.join("webview");
            let id = web.id;
            let sender = self.sender.clone();
            pane = pane.child(
                div().flex_1().min_h_0().child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, _, window, cx| {
                            let mut state = browser.borrow_mut();
                            if state.failed
                                || state.pending
                                || f32::from(bounds.size.width) <= 0.0
                                || f32::from(bounds.size.height) <= 0.0
                            {
                                return;
                            }
                            if state.initialized() {
                                if let Err(error) = state.show(bounds) {
                                    state.failed = true;
                                    let _ = sender
                                        .try_send(Event::BrowserError(id, format!("{error:#}")));
                                }
                                return;
                            }
                            let parent = match crate::browser::NativeParent::capture(window) {
                                Ok(parent) => parent,
                                Err(error) => {
                                    state.failed = true;
                                    let _ = sender
                                        .try_send(Event::BrowserError(id, format!("{error:#}")));
                                    return;
                                }
                            };
                            state.pending = true;
                            drop(state);
                            // Wry construction runs a nested Windows message loop.
                            // An executor task releases the paint's App/Window borrow
                            // before that loop dispatches input, resize or redraw.
                            cx.foreground_executor()
                                .spawn(async move {
                                    let navigation_sender = sender.clone();
                                    let result = crate::browser::Browser::create(
                                        parent,
                                        bounds,
                                        &url,
                                        data,
                                        move |url| {
                                            let _ = navigation_sender
                                                .try_send(Event::BrowserNavigated(id, url));
                                        },
                                    );
                                    match result {
                                        Ok(view) => {
                                            *browser.borrow_mut() = view;
                                            let _ = sender.try_send(Event::BrowserReady(id));
                                        }
                                        Err(error) => {
                                            let mut state = browser.borrow_mut();
                                            state.pending = false;
                                            state.failed = true;
                                            let _ = sender.try_send(Event::BrowserError(
                                                id,
                                                format!("{error:#}"),
                                            ));
                                        }
                                    }
                                })
                                .detach();
                        },
                    )
                    .size_full(),
                ),
            );
        }
        pane.into_any_element()
    }
    fn browser_command(&mut self, script: &str) {
        if matches!(self.preview, Some(PreviewContent::Web(_))) {
            let result = self.browser.borrow_mut().command(script);
            self.session.result(result);
        }
    }
    fn preview(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(PreviewContent::Web(web)) = &self.preview {
            return self.web_preview(web, cx);
        }
        let path = match &self.preview {
            Some(PreviewContent::Code(c)) => Some(&c.file.path),
            Some(PreviewContent::Image(p)) => Some(p),
            Some(PreviewContent::Web(_)) => unreachable!(),
            None => None,
        };
        let name = path
            .map(|p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_else(|| "正在读取文件".into());
        let header = div()
            .flex()
            .items_center()
            .h(px(40.0))
            .flex_shrink_0()
            .text_xs()
            .bg(rgb(PANEL))
            .px_3()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!(
                        "{}{}",
                        name,
                        if self.preview_dirty { " *" } else { "" }
                    )),
            );
        let mut toolbar = div()
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .py_2()
            .flex_shrink_0()
            .child(
                div().flex_1().text_xs().text_color(rgb(MUTED)).child(
                    path.and_then(|p| p.extension())
                        .and_then(|e| e.to_str())
                        .unwrap_or("预览")
                        .to_owned(),
                ),
            );
        if self.preview_editor.is_some() {
            toolbar = toolbar
                .child(Self::button(
                    "wrap-preview",
                    if self.preview_wrap {
                        "换行：开"
                    } else {
                        "换行：关"
                    },
                    cx,
                    |s, w, cx| {
                        s.preview_wrap = !s.preview_wrap;
                        if let Some(input) = &s.preview_editor {
                            input.update(cx, |state, cx| {
                                state.set_soft_wrap(s.preview_wrap, w, cx);
                                state.focus(w, cx);
                            });
                        }
                    },
                ))
                .child(Self::button(
                    "edit-preview",
                    if self.preview_editing {
                        "完成"
                    } else {
                        "编辑"
                    },
                    cx,
                    |s, w, cx| {
                        if s.preview_editing && !s.save_preview(cx) {
                            return;
                        }
                        s.preview_editing = !s.preview_editing;
                        if let Some(input) = &s.preview_editor {
                            input.update(cx, |state, cx| state.focus(w, cx));
                        }
                        cx.notify();
                    },
                ));
        }
        toolbar = toolbar.child(Self::button(
            "copy-preview",
            if matches!(self.preview, Some(PreviewContent::Image(_))) {
                "复制路径"
            } else {
                "复制"
            },
            cx,
            |s, _, cx| {
                let text = if let Some(input) = &s.preview_editor {
                    let state = input.read(cx);
                    let selected = state.selected_value();
                    Some(if selected.is_empty() {
                        state.value().to_string()
                    } else {
                        selected.to_string()
                    })
                } else if let Some(PreviewContent::Image(path)) = &s.preview {
                    Some(display_path(path))
                } else {
                    None
                };
                if let Some(text) = text {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            },
        ));
        let mut pane = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .bg(rgb(BG))
            .child(header.child(Self::tool_button(
                "close-preview",
                "关闭文件预览",
                gpui_kit::assets::IconName::X,
                false,
                cx,
                |s, w, cx| s.request_preview_action(PreviewAction::Close, w, cx),
            )));
        if let Some(path) = path {
            pane = pane.child(
                div()
                    .px_3()
                    .pt_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .truncate()
                    .child(display_path(path)),
            );
        }
        pane = pane.child(toolbar);
        match &self.preview {
            Some(PreviewContent::Web(_)) => unreachable!(),
            Some(PreviewContent::Image(path)) => {
                pane = pane
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .p_4()
                            .child(img(path.clone()).size_full().object_fit(ObjectFit::Contain)),
                    )
                    .child(
                        div()
                            .p_3()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("本地图片预览"),
                    );
            }
            Some(PreviewContent::Code(_)) => {
                if let Some(input) = &self.preview_editor {
                    pane = pane.child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .font_family("Geist Mono")
                            .text_size(px(12.0))
                            .child(
                                CodeEditor::new(input)
                                    .readonly(!self.preview_editing)
                                    .h(relative(1.0))
                                    .bordered(false)
                                    .appearance(false),
                            ),
                    );
                }
                pane = pane.child(div().px_3().py_2().text_xs().text_color(rgb(MUTED)).child(
                    if !self.preview_editing {
                        "只读预览 · 点击右上角编辑"
                    } else if self.preview_dirty {
                        "未保存 · Ctrl+S 保存"
                    } else {
                        "可编辑 · Ctrl+S 保存"
                    },
                ));
            }
            None => {
                pane = pane.child(
                    div()
                        .p_4()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().h_4().w(px(180.0)).rounded_md().bg(rgb(SURFACE)))
                        .child(div().h_4().w(px(260.0)).rounded_md().bg(rgb(SURFACE))),
                );
            }
        }
        pane.into_any_element()
    }
    fn dialog_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(dialog) = self.dialog.clone() else {
            return div().into_any_element();
        };
        let (title, description, confirm) = match dialog {
            Dialog::RenameWorkspace(_) => {
                ("工作区设置", "修改显示名称，不会重命名本地目录。", "保存")
            }
            Dialog::RenameTab(_) => ("重命名终端", "设置便于识别的标签名称。", "保存"),
            Dialog::RemoveWorkspace(_) => (
                "移除工作区",
                "关闭该工作区的终端并移除记录，保留项目文件。",
                "移除",
            ),
            Dialog::CloseTab(_) => ("关闭终端", "终止该标签的进程并移除历史记录。", "关闭终端"),
            Dialog::UnsavedPreview => (
                "文件尚未保存",
                "保存后继续，或明确放弃当前修改。",
                "放弃修改",
            ),
        };
        let mut content = div()
            .flex()
            .flex_col()
            .gap_4()
            .w(px(480.0))
            .p_6()
            .rounded_lg()
            .border_1()
            .border_color(rgb(0x454545))
            .bg(rgb(SURFACE))
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(div().text_sm().text_color(rgb(MUTED)).child(description));
        if let Some(editor) = &self.editor {
            content = content.child(match editor {
                Editor::Single(s) => Input::new(s).into_any_element(),
            });
        }
        let mut actions = div().flex().items_center().gap_2();
        if let Dialog::RenameWorkspace(id) = dialog {
            actions = actions.child(Self::button(
                "remove-workspace",
                "移除工作区",
                cx,
                move |s, w, cx| s.open_dialog(Dialog::RemoveWorkspace(id), w, cx),
            ));
        }
        if matches!(dialog, Dialog::UnsavedPreview) {
            actions = actions.child(Self::primary(
                "save-and-continue",
                "保存并继续",
                cx,
                |s, w, cx| s.finish_pending_preview(true, w, cx),
            ));
        }
        actions = actions
            .child(div().flex_1())
            .child(Self::button("cancel-dialog", "取消", cx, |s, w, cx| {
                s.close_dialog(w, cx)
            }))
            .child(Self::primary(
                "confirm-dialog",
                confirm,
                cx,
                Self::apply_dialog,
            ));
        if let Some(e) = &self.session.error {
            content = content.child(div().text_sm().text_color(rgb(0xefb3a9)).child(e.clone()));
        }
        div()
            .absolute()
            .inset_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000088))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(content.child(actions))
            .into_any_element()
    }
}
impl Render for Ui {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_project_panel();
        if !matches!(self.preview, Some(PreviewContent::Web(_)))
            || !self.panel.open
            || self.panel.tab != PanelTab::Explorer
            || self.settings
            || self.dialog.is_some()
            || self.ssh.modal.is_some()
            || self.new_menu.is_some()
            || self.session.data.workspace().is_none()
        {
            self.browser.borrow_mut().hide();
        }
        self.preview_width = self
            .preview_width
            .min((f32::from(window.viewport_size().width) - SIDEBAR_WIDTH - 324.0).max(280.0));
        let mut content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .bg(rgb(BG));
        if self.settings {
            content = content.child(self.settings_page(cx));
        } else if self.session.data.workspace().is_some() {
            let mut work = div()
                .flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(self.main_area(window, cx));
            if self.panel.open {
                work = work
                    .child(
                        div()
                            .id("preview-divider")
                            .w(px(4.0))
                            .h_full()
                            .flex_shrink_0()
                            .bg(rgb(0x30362b))
                            .hover(|s| s.bg(rgb(ACCENT)))
                            .cursor(CursorStyle::ResizeLeftRight)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|s, event: &MouseDownEvent, _, cx| {
                                    s.resizing_terminal_preview = false;
                                    s.resize_origin = Some((event.position.x, s.preview_width));
                                    cx.stop_propagation();
                                }),
                            ),
                    )
                    .child(self.right_panel(window, cx));
            }
            content = content.child(work);
        } else {
            content = content
                .items_center()
                .justify_center()
                .gap_4()
                .child(icon("terminal").size(px(32.0)).text_color(rgb(MUTED)))
                .child(div().text_lg().child("添加一个项目"))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child("在项目目录中启动 Shell 或 Agent。"),
                )
                .child(Self::primary(
                    "empty-add-workspace",
                    "添加项目",
                    cx,
                    Self::choose_workspace,
                ));
        }
        if self.dialog.is_none() {
            if let Some(error) = &self.session.error {
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .p_3()
                        .bg(rgb(SURFACE))
                        .child(
                            div()
                                .flex_1()
                                .text_sm()
                                .text_color(rgb(0xefb3a9))
                                .child(error.clone()),
                        )
                        .child(Self::button(
                            "dismiss-error",
                            "关闭提示",
                            cx,
                            |s, _, _| s.session.error = None,
                        )),
                );
            }
        }
        div()
            .id("app")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .font_family("Geist")
            .text_size(px(14.0))
            .text_color(rgb(FG))
            .bg(rgb(BG))
            .capture_key_down(cx.listener(Self::on_key))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .child(self.title_bar(window, cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .children((!self.settings).then(|| self.sidebar(cx)))
                    .child(content),
            )
            .child(self.status_bar())
            .children(self.dialog.as_ref().map(|_| self.dialog_view(cx)))
            .children(self.ssh.modal.as_ref().map(|_| self.ssh_dialog_view(cx)))
            .children(self.new_menu.map(|_| self.new_terminal_menu(cx)))
    }
}

struct TerminalSurface {
    owner: Entity<Ui>,
}
impl IntoElement for TerminalSurface {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for TerminalSurface {
    type RequestLayoutState = ();
    type PrepaintState = render::PaintState;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> render::PaintState {
        self.owner.update(cx, |ui, _| {
            ui.bounds = bounds;
            let rows = ((f32::from(bounds.size.height) - 2.0 * INSET) / LINE)
                .floor()
                .clamp(2.0, 250.0) as u16;
            let cols = ((f32::from(bounds.size.width) - 2.0 * INSET) / CELL)
                .floor()
                .clamp(10.0, 500.0) as u16;
            if ui.session.grid != (rows, cols) {
                ui.session.resize(rows, cols);
                ui.refresh();
            }
            let ime_geometry = if ui.focus.is_focused(window) && ui.dialog.is_none() {
                ui.snapshot
                    .ime_bounds(bounds, &ui.preedit, ui.preedit.encode_utf16().count())
                    .map(|caret| (caret, window.scale_factor()))
            } else {
                None
            };
            if ui.last_ime_geometry != ime_geometry {
                ui.last_ime_geometry = ime_geometry;
                if ime_geometry.is_some() {
                    // Schedule after paint has registered the current handler.
                    window.invalidate_character_coordinates();
                }
            }
        });
        let ui = self.owner.read(cx);
        let focused = ui.focus.is_focused(window)
            && ui.dialog.is_none()
            && ui
                .session
                .selected()
                .and_then(|id| ui.session.terminals.get(&id))
                .is_some_and(|t| t.running());
        render::prepare(
            &ui.snapshot,
            bounds,
            ui.selection,
            &ui.preedit,
            ui.session.data.input_cursor_style,
            focused,
            &mut ui.text_cache.borrow_mut(),
            window,
        )
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        paint: &mut render::PaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let ui = self.owner.read(cx);
        if ui.focus.is_focused(window)
            && ui.dialog.is_none()
            && ui
                .session
                .selected()
                .and_then(|id| ui.session.terminals.get(&id))
                .is_some_and(|t| t.running())
        {
            window.handle_input(
                &ui.focus,
                ElementInputHandler::new(bounds, self.owner.clone()),
                cx,
            );
        }
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in paint.quads.drain(..) {
                window.paint_quad(quad);
            }
            for (position, line) in paint.lines.drain(..) {
                let _ = line.paint(position, px(LINE), TextAlign::Left, None, window, cx);
            }
        });
    }
}
impl EntityInputHandler for Ui {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let units: Vec<_> = self.preedit.encode_utf16().collect();
        let start = range.start.min(units.len());
        let end = range.end.min(units.len()).max(start);
        *actual = Some(start..end);
        Some(String::from_utf16_lossy(&units[start..end]))
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.preedit.encode_utf16().count();
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.preedit.is_empty()).then(|| 0..self.preedit.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.preedit.clear();
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        value: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit.clear();
        if self.dialog.is_none()
            && self.ssh.modal.is_none()
            && self.active_document.is_none()
            && !self.content_visible()
        {
            self.session.send(value.as_bytes().to_vec());
            self.selection = None;
        }
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        value: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit = value.into();
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.snapshot.ime_bounds(bounds, &self.preedit, range.start)
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}
fn load_code_preview(link: FileLink) -> Result<CodePreview> {
    Ok(CodePreview {
        file: Preview::load(&link)?,
    })
}
fn display_path(path: &std::path::Path) -> String {
    path.to_string_lossy().trim_start_matches(r"\\?\").into()
}
fn key_bytes(
    key: &str,
    control: bool,
    alt: bool,
    shift: bool,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
    let arrow = |c| {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{c}")
        } else if app_cursor {
            format!("\x1bO{c}")
        } else {
            format!("\x1b[{c}")
        }
    };
    let tilde = |n| {
        if modifier > 1 {
            format!("\x1b[{n};{modifier}~")
        } else {
            format!("\x1b[{n}~")
        }
    };
    let bytes = match key {
        "enter" | "return" => {
            if alt {
                b"\x1b\r".to_vec()
            } else {
                vec![13]
            }
        }
        "backspace" => {
            if alt {
                vec![27, 127]
            } else {
                vec![127]
            }
        }
        "escape" => vec![27],
        "tab" => {
            if shift {
                b"\x1b[Z".to_vec()
            } else {
                vec![9]
            }
        }
        "up" => arrow('A').into_bytes(),
        "down" => arrow('B').into_bytes(),
        "right" => arrow('C').into_bytes(),
        "left" => arrow('D').into_bytes(),
        "home" => arrow('H').into_bytes(),
        "end" => arrow('F').into_bytes(),
        "insert" => tilde(2).into_bytes(),
        "delete" => tilde(3).into_bytes(),
        "pageup" => tilde(5).into_bytes(),
        "pagedown" => tilde(6).into_bytes(),
        k if k.starts_with('f') && k[1..].parse::<u8>().is_ok() => {
            let n = k[1..].parse::<u8>().ok()?;
            match n {
                1..=4 => format!("\x1bO{}", char::from(b'P' + n - 1)).into_bytes(),
                5..=12 => tilde([15, 17, 18, 19, 20, 21, 23, 24][(n - 5) as usize]).into_bytes(),
                _ => return None,
            }
        }
        k if control && k.len() == 1 => {
            let c = k.as_bytes()[0].to_ascii_uppercase();
            if (b'@'..=b'_').contains(&c) {
                vec![c & 0x1f]
            } else {
                return None;
            }
        }
        k if alt && k.len() == 1 => vec![27, k.as_bytes()[0]],
        _ => return None,
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::{key_bytes, load_code_preview};
    use vibe_terminal::paths::FileLink;
    #[cfg(windows)]
    #[test]
    #[ignore = "opens a native GPUI/WebView2 window and DevTools; run explicitly"]
    fn native_web_preview_loads_javascript() {
        use gpui_kit::*;
        use std::{
            io::{Read, Write},
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
            },
            time::{Duration, Instant},
        };
        let root =
            std::env::temp_dir().join(format!("vibe-browser-smoke-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let ready = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let server_ready = ready.clone();
        let server_stop = stop.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(25);
            while !server_stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut buffer = [0; 4096];
                    let count = stream.read(&mut buffer).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..count]);
                    if request.starts_with("GET /ready ") {
                        server_ready.store(true, Ordering::Release);
                    }
                    let body = "<!doctype html><title>Vibe browser smoke</title><h1>WebView2 works</h1><script>fetch('/ready')</script>";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                } else {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        });
        let passed = Arc::new(AtomicBool::new(false));
        let finished = passed.clone();
        let test_root = root.clone();
        gpui_kit::application()
            .with_assets(crate::assets::Assets)
            .run(move |cx| {
                gpui_kit::init(cx);
                let store = vibe_terminal::store::Store {
                    root: test_root.clone(),
                };
                let mut session =
                    vibe_terminal::session::Session::new(store, Default::default(), None);
                session.add_workspace(&test_root);
                let (handle, view) = gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(px(1100.0), px(700.0)),
                            cx,
                        ))),
                        ..Default::default()
                    },
                    cx,
                    |window, cx| {
                        window.set_window_title("Vibe browser click regression");
                        cx.new(|cx| {
                            let mut ui = super::Ui::new(session, window, cx);
                            ui.demo = true;
                            ui
                        })
                    },
                )
                .unwrap();
                cx.spawn(async move |cx| {
                    smol::Timer::after(Duration::from_secs(1)).await;
                    let click_scale = handle.update(cx, |_, window, cx| {
                        view.update(cx, |ui, _| {
                            let root = std::env::temp_dir();
                            let mut buffer = vibe_terminal::terminal::Buffer::new(10, 100, root.clone());
                            buffer.process(url.as_bytes());
                            ui.snapshot = Arc::new(crate::render::Snapshot::read(&buffer, &root));
                            ui.path_press = Some(((0, 0), point(px(300.0), px(120.0))));
                        });
                        window.scale_factor()
                    }).unwrap();
                    // Deliver the actual native mouse-up message, as opposed to
                    // calling the preview action before the first window draw.
                    unsafe {
                        use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_LBUTTONUP};
                        let title: Vec<u16> = "Vibe browser click regression\0".encode_utf16().collect();
                        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
                        assert!(!hwnd.is_null());
                        let x = (300.0 * click_scale) as isize;
                        let y = (120.0 * click_scale) as isize;
                        assert_ne!(PostMessageW(hwnd, WM_LBUTTONUP, 0, x | (y << 16)), 0);
                    }
                    let output_sender = cx.update(|cx| view.read(cx).sender.clone());
                    let traffic = std::thread::spawn(move || {
                        for _ in 0..50 {
                            let _ = output_sender.send_blocking(super::Event::TailscaleAddress(Ok("fixture".into())));
                            std::thread::sleep(Duration::from_millis(2));
                        }
                    });
                    let deadline = Instant::now() + Duration::from_secs(20);
                    while Instant::now() < deadline {
                        smol::Timer::after(Duration::from_millis(100)).await;
                        if ready.load(Ordering::Acquire)
                            && cx.update(|cx| view.read(cx).browser.borrow().initialized())
                        {
                            // Read native child geometry, confirming this is the
                            // right preview panel inside the existing window.
                            handle.update(cx, |_, window, cx| {
                                view.update(cx, |ui, _| {
                                    let browser = ui.browser.borrow();
                                    let bounds = browser.native_bounds();
                                    let position = bounds.position.to_logical::<f64>(window.scale_factor() as f64);
                                    let size = bounds.size.to_logical::<f64>(window.scale_factor() as f64);
                                    eprintln!("native preview: position={position:?}, size={size:?}, viewport={:?}, pane={}", window.viewport_size(), ui.preview_width);
                                    assert!(position.x > 240.0 && position.y >= 48.0);
                                    assert!((size.width - ui.preview_width as f64).abs() <= 2.0);
                                    assert!(position.x + size.width <= f32::from(window.viewport_size().width) as f64 + 2.0);
                                    browser.open_devtools();
                                });
                            }).unwrap();
                            smol::Timer::after(Duration::from_secs(2)).await;
                            handle
                                .update(cx, |_, window, cx| {
                                    view.update(cx, |ui, cx| {
                                        ui.request_preview_action(
                                            super::PreviewAction::Close,
                                            window,
                                            cx,
                                        );
                                    });
                                })
                                .unwrap();
                            finished.store(true, Ordering::Release);
                            break;
                        }
                    }
                    traffic.join().unwrap();
                    cx.update(|cx| cx.quit());
                })
                .detach();
            });
        stop.store(true, Ordering::Relaxed);
        server.join().unwrap();
        assert!(
            passed.load(Ordering::Acquire),
            "WebView2 did not load and execute the fixture JavaScript"
        );
        let _ = std::fs::remove_dir_all(root);
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "opens an isolated native window and checks the Windows IME anchor"]
    fn native_hidden_cursor_ime_anchor_matches_terminal() {
        use gpui_kit::*;
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        use std::time::Duration;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            FindWindowW, SendMessageW, WM_IME_STARTCOMPOSITION,
        };
        #[repr(C)]
        #[derive(Default)]
        struct CandidateForm {
            index: u32,
            style: u32,
            x: i32,
            y: i32,
            area: [i32; 4],
        }
        #[link(name = "imm32")]
        unsafe extern "system" {
            fn ImmCreateContext() -> isize;
            fn ImmAssociateContext(hwnd: *mut std::ffi::c_void, context: isize) -> isize;
            fn ImmDestroyContext(context: isize) -> i32;
            fn ImmGetCandidateWindow(context: isize, index: u32, form: *mut CandidateForm) -> i32;
        }
        let root = std::env::temp_dir().join(format!("vibe-ime-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = vibe_terminal::store::Store { root: root.clone() };
        let mut session = vibe_terminal::session::Session::new(store, Default::default(), None);
        session.add_workspace(&root);
        session.new_tab(vibe_terminal::model::CliProfile::default_shell());
        let passed = Arc::new(AtomicBool::new(false));
        let finished = passed.clone();
        gpui_kit::application().with_assets(crate::assets::Assets).run(move |cx| {
            gpui_kit::init(cx);
            let (handle, view) = gpui_kit::open_window(WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(180.0), px(120.0)),
                    size(px(1100.0), px(700.0)),
                ))),
                ..Default::default()
            }, cx, |window, cx| {
                window.set_window_title("Vibe IME coordinate regression");
                cx.new(|cx| {
                    let mut ui = super::Ui::new(session, window, cx);
                    ui.demo = true;
                    ui
                })
            }).unwrap();
            cx.spawn(async move |cx| {
                smol::Timer::after(Duration::from_secs(1)).await;
                handle.update(cx, |_, window, cx| {
                    view.update(cx, |ui, cx| {
                        let id = ui.session.selected().unwrap();
                        ui.session.terminals[&id].buffer.lock().unwrap().process(b"\x1b[?25l\x1b[8;9H");
                        ui.refresh();
                        ui.preedit = "ni".into();
                        ui.focus.focus(window, cx);
                        assert!(ui.snapshot.cursor.is_none());
                        cx.notify();
                    });
                }).unwrap();
                smol::Timer::after(Duration::from_millis(250)).await;
                let (expected, scale) = handle.update(cx, |_, window, cx| {
                    let ui = view.read(cx);
                    (ui.snapshot.ime_bounds(ui.bounds, &ui.preedit, 2).unwrap(), window.scale_factor())
                }).unwrap();
                // Exercise the actual GPUI -> IMM bridge without typing into or
                // changing the user's input method, clipboard or live sessions.
                unsafe {
                    let title: Vec<u16> = "Vibe IME coordinate regression\0".encode_utf16().collect();
                    let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
                    assert!(!hwnd.is_null());
                    let context = ImmCreateContext();
                    assert_ne!(context, 0);
                    let previous = ImmAssociateContext(hwnd, context);
                    SendMessageW(hwnd, WM_IME_STARTCOMPOSITION, 0, 0);
                    let mut actual = CandidateForm::default();
                    let result = ImmGetCandidateWindow(context, 0, &mut actual);
                    ImmAssociateContext(hwnd, previous);
                    ImmDestroyContext(context);
                    assert_ne!(result, 0, "Windows must receive a candidate anchor even for a hidden cursor");
                    let expected_x = (f32::from(expected.origin.x) * scale) as i32;
                    let expected_y = (f32::from(expected.origin.y) * scale) as i32 + (f32::from(expected.size.height) * scale) as i32 / 2;
                    eprintln!("IME native anchor: ({}, {}), expected: ({expected_x}, {expected_y}), scale={scale}", actual.x, actual.y);
                    assert_eq!((actual.x, actual.y), (expected_x, expected_y));
                }
                handle.update(cx, |_, _, cx| view.update(cx, |ui, _| ui.session.shutdown())).unwrap();
                finished.store(true, Ordering::Release);
                cx.update(|cx| cx.quit());
            }).detach();
        });
        assert!(passed.load(Ordering::Acquire));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminal_control_keys_are_not_plain_text() {
        assert_eq!(
            key_bytes("up", false, false, false, true).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(key_bytes("c", true, false, false, false).unwrap(), [3]);
        assert!(key_bytes("a", false, false, false, false).is_none());
    }
    #[test]
    fn preview_loads_unicode_and_target_location() {
        let path = std::env::temp_dir().join(format!("{}.rs", uuid::Uuid::new_v4()));
        std::fs::write(&path, "fn main() { println!(\"你好\"); }\n").unwrap();
        let p = load_code_preview(FileLink {
            start: 0,
            end: 0,
            path: path.clone(),
            line: Some(1),
            column: None,
        })
        .unwrap();
        assert!(p.file.content.contains("你好"));
        assert_eq!(p.file.line, Some(1));
        std::fs::remove_file(path).unwrap();
    }
}
