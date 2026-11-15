use crate::render::{self, CELL, INSET, LINE, Snapshot};
use anyhow::{Result, bail};
use gpui_kit::{
    component::{
        IconName, Sizable,
        button::*,
        input::{Editor as CodeEditor, EditorState, Input, InputEvent, InputState, Position},
    },
    *,
};
use std::{
    ops::Range,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;
use vibe_terminal::{
    model::{CliKind, CliProfile, WorkspacePage},
    paths::{FileLink, Preview},
    scroll::WheelAccumulator,
    session::Session,
};

const BG: u32 = 0x181818;
const PANEL: u32 = 0x1f1f1f;
const SURFACE: u32 = 0x272727;
const FG: u32 = 0xdddddf;
const MUTED: u32 = 0x929297;
const ACCENT: u32 = 0xa8d5a2;

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
    File(FileLink),
    Image(PathBuf),
    Workspace(Uuid),
    AddWorkspace(PathBuf),
    RemoveWorkspace(Uuid),
    Quit,
}
enum Editor {
    Single(Entity<InputState>),
}
enum PreviewContent {
    Code(CodePreview),
    Image(PathBuf),
}
struct CodePreview {
    file: Preview,
}

#[derive(Clone, Copy, PartialEq)]
enum SettingsTab {
    Agents,
    Shortcuts,
    Remote,
}
impl SettingsTab {
    fn title(self) -> &'static str {
        match self {
            Self::Agents => "Agent",
            Self::Shortcuts => "快捷键",
            Self::Remote => "Remote",
        }
    }
}
enum Event {
    Output,
    Preview(Uuid, Result<PreviewContent>),
    Web(vibe_terminal::web::Request),
    TailscaleAddress(Result<String>),
}

pub struct Ui {
    pub session: Session,
    focus: FocusHandle,
    snapshot: Arc<Snapshot>,
    bounds: Bounds<Pixels>,
    selection: Option<(CellPoint, CellPoint)>,
    selecting: bool,
    path_press: Option<(CellPoint, Point<Pixels>)>,
    preedit: String,
    settings: bool,
    settings_tab: SettingsTab,
    expanded_workspaces: Vec<Uuid>,
    transcript: Option<vibe_terminal::transcript::Transcript>,
    detected_agents: Vec<vibe_terminal::agents::Detected>,
    wheel: WheelAccumulator,
    dialog: Option<Dialog>,
    editor: Option<Editor>,
    preview: Option<PreviewContent>,
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
        if session.page() == WorkspacePage::Terminal {
            session.ensure_history();
        }
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
                                ui.session.take_dirty();
                                ui.refresh();
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
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut ui = Self {
            session,
            focus,
            snapshot: Arc::default(),
            bounds: Bounds::default(),
            selection: None,
            selecting: false,
            path_press: None,
            preedit: String::new(),
            settings: false,
            settings_tab: SettingsTab::Agents,
            expanded_workspaces: Vec::new(),
            transcript: None,
            detected_agents: Vec::new(),
            new_menu: None,
            wheel: WheelAccumulator::default(),
            dialog: None,
            editor: None,
            preview: None,
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
            _tasks: vec![events, checkpoint],
            demo: false,
        };
        ui.refresh();
        // 启动时自动恢复上次选中的会话，避免“历史已加载但进程未启动”的状态。
        if let Some(id) = ui.session.selected()
            && !ui
                .session
                .terminals
                .get(&id)
                .is_some_and(|terminal| terminal.running())
        {
            ui.session.start(id);
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
        if let (Some(t), Some(w)) = (
            self.session
                .selected()
                .and_then(|id| self.session.terminals.get(&id)),
            self.session.data.workspace(),
        ) {
            if let Ok(b) = t.buffer.lock() {
                self.snapshot = Arc::new(Snapshot::read(&b, &w.directory));
                return;
            }
        }
        self.snapshot = Arc::default();
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
        self.preview = None;
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
                self.focus.focus(window, cx);
            }
            PreviewAction::File(link) => {
                self.clear_preview();
                self.open_file(link);
            }
            PreviewAction::Image(path) => {
                self.clear_preview();
                self.load_image(path);
            }
            PreviewAction::Workspace(id) => {
                self.clear_preview();
                self.select_workspace(id);
            }
            PreviewAction::AddWorkspace(path) => {
                self.clear_preview();
                self.session.add_workspace(&path);
                self.refresh();
            }
            PreviewAction::RemoveWorkspace(id) => {
                self.clear_preview();
                self.session.remove_workspace(id);
                self.refresh();
            }
            PreviewAction::Quit => cx.quit(),
        }
        cx.notify();
    }
    fn finish_pending_preview(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        if save && !self.save_preview(cx) {
            return;
        }
        let action = self.pending_preview.take();
        self.preview_dirty = false;
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
            true
        }
    }
    fn select_workspace(&mut self, id: Uuid) {
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
        self.wheel.reset();
        self.session.select_tab(id);
        self.selection = None;
        self.preedit.clear();
        // 切到休眠标签时直接恢复会话，不再需要单独的“恢复”按钮。
        if !self
            .session
            .terminals
            .get(&id)
            .is_some_and(|terminal| terminal.running())
        {
            self.session.start(id);
        }
        self.refresh();
        self.focus.focus(window, cx);
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
    fn toggle_transcript(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.transcript.is_some() {
            self.transcript = None;
            self.focus.focus(window, cx);
            return;
        }
        let Some(tab) = self
            .session
            .selected()
            .and_then(|id| self.session.data.tab(id))
            .cloned()
        else {
            return;
        };
        let root = self.session.store.root.clone();
        match vibe_terminal::transcript::locate(&tab, &root) {
            Some(path) => match vibe_terminal::transcript::read(&path) {
                Ok(transcript) => self.transcript = Some(transcript),
                Err(error) => self.session.error = Some(format!("无法读取会话文件：{error:#}")),
            },
            None => self.session.error = Some(
                "当前标签没有可读的会话文件：目前支持 Pi 与 Codex（Claude 本机未留下 transcript）"
                    .to_owned(),
            ),
        }
    }
    fn transcript_view(
        &self,
        transcript: &vibe_terminal::transcript::Transcript,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut list = div()
            .id("transcript")
            .flex()
            .flex_col()
            .gap_3()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_4();
        for message in &transcript.messages {
            let color = match message.role {
                vibe_terminal::transcript::Role::User => ACCENT,
                vibe_terminal::transcript::Role::Assistant => FG,
                vibe_terminal::transcript::Role::Tool => MUTED,
                vibe_terminal::transcript::Role::System => MUTED,
            };
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(color))
                            .child(message.role.label()),
                    )
                    .child(div().text_sm().child(message.text.clone())),
            );
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .bg(rgb(PANEL))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                        "{} 条消息 · {}",
                        transcript.messages.len(),
                        transcript.source.display()
                    )))
                    .child(div().flex_1())
                    .child(Self::button(
                        "refresh-transcript",
                        "刷新",
                        cx,
                        |s, window, cx| {
                            s.transcript = None;
                            s.toggle_transcript(window, cx);
                        },
                    ))
                    .child(Self::button(
                        "close-transcript",
                        "返回终端",
                        cx,
                        |s, window, cx| {
                            s.transcript = None;
                            s.focus.focus(window, cx);
                        },
                    )),
            )
            .child(list)
            .into_any_element()
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
            .ghost()
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
        self.new_terminal_with(CliProfile::default_shell(), window, cx);
    }
    fn new_terminal_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some((x, y)) = self.new_menu else {
            return div().into_any_element();
        };
        let mut panel = div()
            .absolute()
            .left(px(x))
            .top(px(y + 6.0))
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
            panel = panel.child(
                div()
                    .id(SharedString::from(format!("new-terminal-{}", profile.id)))
                    .w_full()
                    .flex()
                    .items_center()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .on_click(cx.listener(move |s, _, window, cx| {
                        s.new_terminal_with(profile.clone(), window, cx);
                        cx.notify();
                    }))
                    .child(div().text_sm().child(name)),
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
            .w(px(180.0))
            .h_full()
            .p_4()
            .bg(rgb(PANEL))
            .child(div().text_xs().text_color(rgb(MUTED)).pb_2().child("设置"));
        for tab in [
            SettingsTab::Agents,
            SettingsTab::Shortcuts,
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
        let nav = nav.child(div().flex_1()).child(
            div()
                .id("close-settings")
                .w_full()
                .flex()
                .items_center()
                .px_2()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                .text_color(rgb(MUTED))
                .on_click(cx.listener(|s, _, window, cx| {
                    s.settings = false;
                    s.focus.focus(window, cx);
                    cx.notify();
                }))
                .child(div().text_sm().child("返回工作区")),
        );
        let body = match self.settings_tab {
            SettingsTab::Agents => self.agents_pane(cx),
            SettingsTab::Shortcuts => self.shortcuts_pane(),
            SettingsTab::Remote => self.web_settings(cx),
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
    fn choose_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择图片".into()),
        });
        cx.spawn_in(window, async move |weak, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                if let Some(path) = paths.into_iter().next() {
                    let _ = weak.update_in(cx, |ui, window, cx| {
                        ui.request_preview_action(PreviewAction::Image(path), window, cx);
                    });
                }
            }
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
                .map(|t| t.title.clone())
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
        if restore_file && let Some(input) = &self.preview_editor {
            input.update(cx, |state, cx| state.focus(window, cx));
        } else {
            self.focus.focus(window, cx);
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
                    t.title = value;
                }
            }
            Dialog::RemoveWorkspace(id) => {
                if self.preview_dirty {
                    self.request_preview_action(PreviewAction::RemoveWorkspace(id), window, cx);
                    return;
                }
                self.session.remove_workspace(id);
                self.preview = None;
                self.preview_request = None;
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
        if self.transcript.is_some() && key == "escape" {
            self.transcript = None;
            self.focus.focus(window, cx);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.settings && key == "escape" {
            self.settings = false;
            self.editor = None;
            self.focus.focus(window, cx);
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
                self.focus.focus(window, cx);
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if let Some(id) = self.session.selected() {
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
                let target = self
                    .session
                    .data
                    .workspace()
                    .and_then(|workspace| workspace.tabs.get(index as usize - 1))
                    .map(|tab| tab.id);
                if let Some(id) = target {
                    self.select_tab(id, window, cx);
                }
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }
        if m.control && key == "tab" {
            if let Some(w) = self.session.data.workspace() {
                if !w.tabs.is_empty() {
                    let index = w
                        .tabs
                        .iter()
                        .position(|t| Some(t.id) == w.selected_tab)
                        .unwrap_or(0);
                    let next = if m.shift {
                        (index + w.tabs.len() - 1) % w.tabs.len()
                    } else {
                        (index + 1) % w.tabs.len()
                    };
                    let id = w.tabs[next].id;
                    self.select_tab(id, window, cx);
                }
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if !self.focus.is_focused(window) || self.session.page() != WorkspacePage::Terminal {
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
            self.session.send(bytes);
            self.selection = None;
            cx.stop_propagation();
            cx.notify();
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
        } else {
            self.selection = Some((point, point));
            self.selecting = true;
            self.path_press = None;
        }
        cx.notify();
    }
    fn mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some((origin, width)) = self.resize_origin {
            self.preview_width = (width - f32::from(event.position.x - origin)).clamp(280.0, 800.0);
            cx.notify();
            return;
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
        if let Some((point, start)) = self.path_press.take() {
            if (f32::from(event.position.x - start.x)).abs() < 4.0
                && (f32::from(event.position.y - start.y)).abs() < 4.0
            {
                if let Some(link) = self.snapshot.link_at(point.0, point.1).cloned() {
                    self.request_preview_action(PreviewAction::File(link), window, cx);
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
        for w in &self.session.data.workspaces {
            let id = w.id;
            let active = Some(id) == self.session.data.selected_workspace;
            let expanded = self.expanded_workspaces.contains(&id);
            let mut badges = div().flex().flex_wrap().gap_1().px_2().pb_1();
            for tab in &w.tabs {
                let (glyph, color) = profile_badge(&tab.profile);
                let running = self
                    .session
                    .terminals
                    .get(&tab.id)
                    .is_some_and(|terminal| terminal.running());
                badges = badges.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(20.0))
                        .h(px(20.0))
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(if running { color } else { 0x3a3a3a }))
                        .text_xs()
                        .text_color(rgb(if running { color } else { MUTED }))
                        .child(glyph),
                );
            }
            let mut item = div()
                .id(SharedString::from(format!("workspace-{id}")))
                .flex()
                .flex_col()
                .rounded_md()
                .p_1()
                .bg(rgb(if active { SURFACE } else { PANEL }))
                .cursor_pointer()
                .on_click(cx.listener(move |s, _, window, cx| {
                    s.request_workspace(id, window, cx);
                    cx.notify();
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(Self::button(
                            SharedString::from(format!("workspace-toggle-{id}")),
                            if expanded { "▾" } else { "▸" },
                            cx,
                            move |s, _, _| s.toggle_workspace_expanded(id),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_align(TextAlign::Left)
                                .child(w.name.clone()),
                        )
                        .child(Self::button(
                            SharedString::from(format!("workspace-settings-{id}")),
                            "···",
                            cx,
                            move |s, window, cx| {
                                s.open_dialog(Dialog::RenameWorkspace(id), window, cx)
                            },
                        )),
                )
                .child(badges);
            if expanded {
                let mut rows = div().flex().flex_col().gap_1().pl_4().pb_1();
                for tab in &w.tabs {
                    let tab_id = tab.id;
                    let (glyph, color) = profile_badge(&tab.profile);
                    let running = self
                        .session
                        .terminals
                        .get(&tab_id)
                        .is_some_and(|terminal| terminal.running());
                    rows = rows.child(
                        div()
                            .id(SharedString::from(format!("workspace-tab-{tab_id}")))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0x313131)))
                            .on_click(cx.listener(move |s, _, window, cx| {
                                s.open_tab(id, tab_id, window, cx);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .w(px(20.0))
                                    .text_xs()
                                    .text_color(rgb(color))
                                    .child(glyph),
                            )
                            .child(div().flex_1().min_w_0().truncate().child(tab.title.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(if running { ACCENT } else { MUTED }))
                                    .child(if running { "●" } else { "◌" }),
                            ),
                    );
                }
                item = item.child(rows);
            }
            list = list.child(item);
        }
        div()
            .flex()
            .flex_col()
            .w(px(240.0))
            .h_full()
            .flex_shrink_0()
            .p_4()
            .gap_4()
            .bg(rgb(PANEL))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(40.0))
                    .child(div().text_lg().text_color(rgb(ACCENT)).child("V"))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Vibe Terminal"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("工作区")
                    .child(Self::button(
                        "add-workspace-small",
                        "＋",
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
            .child(
                Button::new("settings")
                    .icon(IconName::Settings)
                    .small()
                    .ghost()
                    .accessibility_label("设置")
                    .tooltip("设置")
                    .on_click(cx.listener(|s, _, window, cx| {
                        s.open_settings(window, cx);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
    fn overview(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(w) = self.session.data.workspace() else {
            return div().into_any_element();
        };
        let tabs = w.tabs.len();
        let running = w
            .tabs
            .iter()
            .filter(|t| {
                self.session
                    .terminals
                    .get(&t.id)
                    .is_some_and(|t| t.running())
            })
            .count();
        let card = |title: &'static str, value: String, detail: &'static str| {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .p_4()
                .gap_2()
                .rounded_lg()
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(SURFACE))
                .child(div().text_xs().text_color(rgb(MUTED)).child(title))
                .child(
                    div()
                        .text_2xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(value),
                )
                .child(div().text_xs().text_color(rgb(MUTED)).child(detail))
        };
        div()
            .id("overview-page")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_6()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(w.name.clone()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child("一个项目，一处工作空间。"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(card("终端标签", tabs.to_string(), "按项目独立保存"))
                    .child(card("正在运行", running.to_string(), "关闭应用后全部退出"))
                    .child(card(
                        "尚未加载",
                        w.tabs
                            .iter()
                            .filter(|t| {
                                !self
                                    .session
                                    .terminals
                                    .get(&t.id)
                                    .is_some_and(|t| t.loaded())
                            })
                            .count()
                            .to_string(),
                        "需要时再恢复会话",
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(SURFACE))
                    .bg(rgb(PANEL))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("项目目录"))
                    .child(
                        div()
                            .font_family("Geist Mono")
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(display_path(&w.directory)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(Self::primary(
                                "overview-new-terminal",
                                "＋ 新终端",
                                cx,
                                |s, w, cx| s.new_terminal(w, cx),
                            ))
                            .child(Self::button(
                                "overview-image",
                                "打开图片",
                                cx,
                                Self::choose_image,
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(SURFACE))
                    .bg(rgb(PANEL))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("继续工作"))
                    .children(w.tabs.iter().map(|tab| {
                        let id = tab.id;
                        Self::button(
                            SharedString::from(format!("overview-tab-{id}")),
                            format!("{}   ·   {}", tab.title, tab.profile.name),
                            cx,
                            move |s, w, cx| s.select_tab(id, w, cx),
                        )
                    })),
            )
            .into_any_element()
    }
    fn terminal(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(transcript) = &self.transcript {
            return self.transcript_view(transcript, cx);
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
                .cursor(CursorStyle::IBeam)
                .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
                .on_mouse_move(cx.listener(Self::mouse_move))
                .on_scroll(cx.listener(Self::wheel))
                .child(TerminalSurface { owner: cx.entity() }),
        )
        .into_any_element()
    }
    fn preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let path = match &self.preview {
            Some(PreviewContent::Code(c)) => Some(&c.file.path),
            Some(PreviewContent::Image(p)) => Some(p),
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
        let mut header = div()
            .flex()
            .items_center()
            .h(px(48.0))
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
        if self.preview_editor.is_some() {
            header = header
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
        header = header
            .child(Self::button(
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
            ))
            .child(Self::button("close-preview", "×", cx, |s, w, cx| {
                s.request_preview_action(PreviewAction::Close, w, cx)
            }));
        let mut pane = div()
            .flex()
            .flex_col()
            .w(px(self.preview_width))
            .h_full()
            .flex_shrink_0()
            .min_w_0()
            .bg(rgb(PANEL))
            .child(header);
        if let Some(path) = path {
            pane = pane.child(
                div()
                    .px_3()
                    .pb_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .truncate()
                    .child(display_path(path)),
            );
        }
        match &self.preview {
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
                            .text_size(px(14.0))
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
                "关闭该工作区的终端并移除记录，保留本地项目文件。",
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
        self.preview_width = self
            .preview_width
            .min((f32::from(window.viewport_size().width) - 564.0).max(280.0));
        let mut content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(BG));
        if self.settings {
            content = content.child(self.settings_page(cx));
        } else if let Some(w) = self.session.data.workspace() {
            let mut tabs = div().flex().gap_1().items_center().p_2();
            tabs = tabs.child(Self::button(
                "overview-tab",
                "项目概览",
                cx,
                |s, _, _| {
                    s.session.overview();
                    s.selection = None;
                },
            ));
            for tab in &w.tabs {
                let id = tab.id;
                let active = w.page == WorkspacePage::Terminal && w.selected_tab == Some(id);
                let status = if self.session.terminals.get(&id).is_some_and(|t| t.running()) {
                    "●"
                } else {
                    "◌"
                };
                tabs = tabs.child(
                    div()
                        .flex()
                        .items_center()
                        .rounded_md()
                        .bg(rgb(if active { 0x313131 } else { PANEL }))
                        .child(
                            div()
                                .id(SharedString::from(format!("tab-{id}")))
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_2()
                                .py_1()
                                .cursor_pointer()
                                .on_click(cx.listener(move |s, event: &ClickEvent, w, cx| {
                                    if event.click_count() == 2 {
                                        s.open_dialog(Dialog::RenameTab(id), w, cx);
                                    } else {
                                        s.select_tab(id, w, cx);
                                    }
                                    cx.notify();
                                }))
                                .child({
                                    let (glyph, color) = profile_badge(&tab.profile);
                                    div()
                                        .w(px(18.0))
                                        .text_xs()
                                        .text_color(rgb(color))
                                        .child(glyph)
                                })
                                .child(div().text_sm().child(format!("{status}  {}", tab.title))),
                        )
                        .child(Self::button(
                            SharedString::from(format!("close-{id}")),
                            "×",
                            cx,
                            move |s, w, cx| s.open_dialog(Dialog::CloseTab(id), w, cx),
                        )),
                );
            }
            tabs = tabs.child(
                Button::new("new-terminal")
                    .icon(IconName::Plus)
                    .small()
                    .ghost()
                    .accessibility_label("新建终端")
                    .tooltip("新建终端")
                    .on_click(cx.listener(|s, event: &ClickEvent, _, cx| {
                        let origin = event.position();
                        s.detected_agents = vibe_terminal::agents::detected();
                        s.new_menu = Some((origin.x.into(), origin.y.into()));
                        cx.notify();
                    })),
            );
            let transcript_ready = self.transcript.is_some()
                || w.selected_tab
                    .and_then(|id| w.tabs.iter().find(|tab| tab.id == id))
                    .is_some_and(|tab| matches!(tab.profile.kind, CliKind::Pi | CliKind::Codex));
            if transcript_ready {
                tabs = tabs.child(Self::button(
                    "toggle-transcript",
                    if self.transcript.is_some() {
                        "终端"
                    } else {
                        "内容"
                    },
                    cx,
                    |s, window, cx| {
                        s.toggle_transcript(window, cx);
                        cx.notify();
                    },
                ));
            }
            content = content.child(
                div()
                    .id("tabs")
                    .h(px(48.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .pr_2()
                    .flex_shrink_0()
                    .bg(rgb(PANEL))
                    .child(
                        div()
                            .id("tab-strip")
                            .flex_1()
                            .min_w_0()
                            .overflow_x_scroll()
                            .child(tabs),
                    ),
            );
            let page = match self.session.page() {
                WorkspacePage::Overview => self.overview(cx),
                WorkspacePage::Terminal => self.terminal(cx),
            };
            let mut work = div().flex().flex_1().min_h_0().min_w_0().child(page);
            if self.preview.is_some() || self.preview_request.is_some() {
                work = work
                    .child(
                        div()
                            .id("preview-divider")
                            .w(px(4.0))
                            .h_full()
                            .flex_shrink_0()
                            .bg(rgb(SURFACE))
                            .cursor(CursorStyle::ResizeLeftRight)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|s, event: &MouseDownEvent, _, cx| {
                                    s.resize_origin = Some((event.position.x, s.preview_width));
                                    cx.stop_propagation();
                                }),
                            ),
                    )
                    .child(self.preview(cx));
            }
            content = content.child(work);
        } else {
            content = content
                .items_center()
                .justify_center()
                .gap_6()
                .child(
                    div()
                        .text_3xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("从一个项目开始"),
                )
                .child(
                    div()
                        .text_color(rgb(MUTED))
                        .child("选择本地目录，为 AI Coding 创建独立的工作区。"),
                )
                .child(Self::primary(
                    "empty-add-workspace",
                    "＋ 创建工作区",
                    cx,
                    Self::choose_workspace,
                ))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("Claude Code  /  Codex  /  Pi  /  Shell"),
                );
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
            .font_family("Geist")
            .text_size(px(14.0))
            .text_color(rgb(FG))
            .bg(rgb(BG))
            .capture_key_down(cx.listener(Self::on_key))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .children((!self.settings).then(|| self.sidebar(cx)))
            .child(content)
            .children(self.dialog.as_ref().map(|_| self.dialog_view(cx)))
            .children(self.new_menu.map(|_| self.new_terminal_menu(cx)))
    }
}

/// Agent 徽标：一个短标识 + 颜色，用于侧边栏与标签栏。
fn profile_badge(profile: &CliProfile) -> (String, u32) {
    match profile.kind {
        CliKind::Shell => {
            let command = profile.command.to_ascii_lowercase();
            if command.contains("pwsh") || command.contains("powershell") {
                ("PS".to_owned(), 0x6fa8dc)
            } else {
                (">_".to_owned(), 0x9aa0a6)
            }
        }
        CliKind::Claude => ("C".to_owned(), 0xd97757),
        CliKind::Codex => ("X".to_owned(), 0x10a37f),
        CliKind::Pi => ("π".to_owned(), ACCENT),
        CliKind::Custom => {
            let initial = profile
                .name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "?".to_owned());
            (initial, MUTED)
        }
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
            focused,
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
        if self.dialog.is_none() && self.session.page() == WorkspacePage::Terminal {
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
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (row, col) = self.snapshot.cursor?;
        Some(Bounds::new(
            self.bounds.origin
                + point(px(INSET + col as f32 * CELL), px(INSET + row as f32 * LINE)),
            size(px(CELL), px(LINE)),
        ))
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
