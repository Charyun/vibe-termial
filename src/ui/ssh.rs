#[cfg(all(test, windows))]
#[path = "ssh_native_test.rs"]
mod native_test;
use super::*;
use vibe_terminal::ssh::SshHost;

#[derive(Default)]
pub(super) struct SshUi {
    pub modal: Option<SshModal>,
    pending: HashMap<Uuid, Uuid>,
    results: HashMap<Uuid, std::result::Result<String, String>>,
    notice: Option<String>,
}
pub(super) enum SshModal {
    Workspace {
        remote: bool,
        host: Option<Uuid>,
        path: Entity<InputState>,
        name: Entity<InputState>,
        error: Option<String>,
    },
    Host(HostForm),
}
pub(super) struct HostForm {
    id: Uuid,
    name: Entity<InputState>,
    host: Entity<InputState>,
    user: Entity<InputState>,
    port: Entity<InputState>,
    key: Entity<InputState>,
    config: Entity<InputState>,
    error: Option<String>,
}
fn input(
    value: &str,
    placeholder: &str,
    window: &mut Window,
    cx: &mut Context<Ui>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(value)
            .placeholder(placeholder)
    })
}
fn field(label: &str, state: &Entity<InputState>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .min_w_0()
        .child(
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(label.to_owned()),
        )
        .child(Input::new(state))
        .into_any_element()
}
fn path_value(state: &Entity<InputState>, cx: &App) -> Option<PathBuf> {
    let text = state.read(cx).value();
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        if let Some(base) = directories::BaseDirs::new() {
            return Some(base.home_dir().join(rest));
        }
    }
    Some(text.into())
}
impl Ui {
    pub(super) fn open_workspace_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.blur(cx);
        self.ssh.modal = Some(SshModal::Workspace {
            remote: false,
            host: None,
            path: input("~", "/home/user/project 或 ~/project", window, cx),
            name: input("", "默认使用目录名称", window, cx),
            error: None,
        });
        self.new_menu = None;
        cx.notify();
    }
    fn edit_ssh_host(&mut self, id: Option<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        let host = id
            .and_then(|id| self.session.data.ssh_hosts.iter().find(|h| h.id == id))
            .cloned();
        self.ssh.modal = Some(SshModal::Host(HostForm {
            id: host.as_ref().map(|h| h.id).unwrap_or_else(Uuid::new_v4),
            name: input(
                host.as_ref().map(|h| h.name.as_str()).unwrap_or(""),
                "例如开发服务器",
                window,
                cx,
            ),
            host: input(
                host.as_ref().map(|h| h.host.as_str()).unwrap_or(""),
                "IP、域名或 SSH 配置别名",
                window,
                cx,
            ),
            user: input(
                host.as_ref().map(|h| h.user.as_str()).unwrap_or(""),
                "留空使用 SSH 配置或系统用户名",
                window,
                cx,
            ),
            port: input(
                &host
                    .as_ref()
                    .and_then(|h| h.port)
                    .map(|p| p.to_string())
                    .unwrap_or_default(),
                "留空使用 SSH 配置，默认 22",
                window,
                cx,
            ),
            key: input(
                &host
                    .as_ref()
                    .and_then(|h| h.identity_file.as_ref())
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                "留空使用 SSH Agent 或默认密钥",
                window,
                cx,
            ),
            config: input(
                &host
                    .as_ref()
                    .and_then(|h| h.config_file.as_ref())
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                "留空使用 ~/.ssh/config",
                window,
                cx,
            ),
            error: None,
        }));
        if let Some(SshModal::Host(form)) = &self.ssh.modal {
            form.name.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }
    fn save_ssh_host(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(SshModal::Host(form)) = &self.ssh.modal else {
            return;
        };
        let result = (|| -> Result<SshHost> {
            let port = form.port.read(cx).value();
            let port = port.trim();
            let host = SshHost {
                id: form.id,
                name: form.name.read(cx).value().trim().into(),
                host: form.host.read(cx).value().trim().into(),
                user: form.user.read(cx).value().trim().into(),
                port: if port.is_empty() {
                    None
                } else {
                    Some(
                        port.parse::<u16>()
                            .map_err(|_| anyhow::anyhow!("端口必须为 1–65535"))?,
                    )
                },
                identity_file: path_value(&form.key, cx),
                config_file: path_value(&form.config, cx),
            };
            host.validate()?;
            Ok(host)
        })();
        match result {
            Ok(host) => {
                let id = host.id;
                if let Some(existing) = self.session.data.ssh_hosts.iter_mut().find(|h| h.id == id)
                {
                    *existing = host;
                } else {
                    self.session.data.ssh_hosts.push(host);
                }
                self.ssh.results.remove(&id);
                self.ssh.pending.remove(&id);
                self.session.persist();
                self.ssh.modal = None;
                self.focus.focus(window, cx);
            }
            Err(error) => {
                if let Some(SshModal::Host(form)) = &mut self.ssh.modal {
                    form.error = Some(error.to_string());
                }
            }
        }
        cx.notify();
    }
    fn connect_remote_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(SshModal::Workspace {
            host, path, name, ..
        }) = &self.ssh.modal
        else {
            return;
        };
        let result = (|| -> Result<(Uuid, String, String)> {
            let host = host.ok_or_else(|| anyhow::anyhow!("请选择一个远端主机"))?;
            let path = path.read(cx).value().trim().to_owned();
            vibe_terminal::ssh::validate_project_path(&path)?;
            Ok((host, path, name.read(cx).value().trim().to_owned()))
        })();
        match result {
            Ok((host_id, path, name)) => {
                self.ssh.modal = None;
                self.request_preview_action(
                    PreviewAction::AddRemoteWorkspace {
                        host_id,
                        path,
                        name,
                    },
                    window,
                    cx,
                );
            }
            Err(error) => {
                if let Some(SshModal::Workspace { error: message, .. }) = &mut self.ssh.modal {
                    *message = Some(error.to_string());
                }
            }
        }
        cx.notify();
    }
    fn probe_ssh_host(&mut self, host: SshHost, cx: &mut Context<Self>) {
        let id = host.id;
        let request = Uuid::new_v4();
        self.ssh.pending.insert(id, request);
        self.ssh.results.remove(&id);
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send_blocking(Event::SshProbe(
                request,
                id,
                vibe_terminal::ssh::probe(&host),
            ));
        });
        cx.notify();
    }
    pub(super) fn accept_ssh_probe(&mut self, request: Uuid, id: Uuid, result: Result<String>) {
        if self.ssh.pending.get(&id) != Some(&request) {
            return;
        }
        self.ssh.pending.remove(&id);
        self.ssh
            .results
            .insert(id, result.map_err(|e| format!("{e:#}")));
    }
    fn remove_ssh_host(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self
            .session
            .data
            .workspaces
            .iter()
            .any(|w| w.remote.as_ref().is_some_and(|r| r.host_id == id))
        {
            self.ssh.notice = Some("该主机仍被远程工作区使用，请先移除对应工作区记录。".into());
        } else {
            self.session.data.ssh_hosts.retain(|h| h.id != id);
            self.ssh.pending.remove(&id);
            self.ssh.results.remove(&id);
            self.session.persist();
            self.ssh.notice = None;
        }
        cx.notify();
    }
    pub(super) fn ssh_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut pane=div().flex().flex_col().gap_4()
            .child(div().flex().items_center().gap_3().child(div().flex_1().text_sm().text_color(rgb(MUTED)).child("添加可连接的 SSH 主机，再在创建工作区时选择。支持 Linux / macOS 的 POSIX Shell。"))
                .child(Self::primary("add-ssh-host","添加主机",cx,|s,w,cx|s.edit_ssh_host(None,w,cx))));
        if self.session.data.ssh_hosts.is_empty() {
            pane = pane.child(
                div()
                    .p_4()
                    .rounded_md()
                    .bg(rgb(PANEL))
                    .text_sm()
                    .child("还没有添加 SSH 主机"),
            );
        }
        for host in &self.session.data.ssh_hosts {
            let id = host.id;
            let copy = host.clone();
            let pending = self.ssh.pending.contains_key(&id);
            let mut row = div()
                .p_4()
                .rounded_md()
                .border_1()
                .border_color(rgb(SURFACE))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon("server"))
                        .child(div().flex_1().min_w_0().child(host.name.clone()))
                        .child(Self::button(
                            SharedString::from(format!("probe-ssh-{id}")),
                            if pending {
                                "连接中…"
                            } else {
                                "测试连接"
                            },
                            cx,
                            move |s, _, cx| {
                                if !s.ssh.pending.contains_key(&id) {
                                    s.probe_ssh_host(copy.clone(), cx);
                                }
                            },
                        ))
                        .child(Self::button(
                            SharedString::from(format!("edit-ssh-{id}")),
                            "编辑",
                            cx,
                            move |s, w, cx| s.edit_ssh_host(Some(id), w, cx),
                        ))
                        .child(Self::button(
                            SharedString::from(format!("remove-ssh-{id}")),
                            "移除",
                            cx,
                            move |s, _, cx| s.remove_ssh_host(id, cx),
                        )),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(host.destination()),
                );
            if let Some(result) = self.ssh.results.get(&id) {
                row = row.child(
                    div()
                        .text_xs()
                        .text_color(rgb(if result.is_ok() { ACCENT } else { 0xefb3a9 }))
                        .child(match result {
                            Ok(s) | Err(s) => s.clone(),
                        }),
                );
            }
            pane = pane.child(row);
        }
        if let Some(notice) = &self.ssh.notice {
            pane = pane.child(
                div()
                    .text_sm()
                    .text_color(rgb(0xefb3a9))
                    .child(notice.clone()),
            );
        }
        pane.child(div().text_xs().text_color(rgb(MUTED)).child("使用本机 OpenSSH 与现有 SSH 配置。首次主机确认和密码/私钥口令可在连接后的终端输入；连接测试使用非交互认证。修改主机后，新建或重连的终端使用新配置。")).into_any_element()
    }
    pub(super) fn ssh_dialog_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(modal) = &self.ssh.modal else {
            return div().into_any_element();
        };
        let mut body = div()
            .id("ssh-modal-scroll")
            .flex()
            .flex_col()
            .gap_3()
            .p_6()
            .w(px(560.0))
            .max_h(relative(0.9))
            .overflow_y_scroll()
            .rounded_lg()
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(0x454545));
        let primary_action: Button;
        match modal {
            SshModal::Workspace {
                remote,
                host,
                path,
                name,
                error,
            } => {
                body = body.child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("创建工作区"),
                );
                let mut modes = div().flex().gap_2();
                for (is_remote, label, glyph) in [
                    (false, "本地项目", gpui_kit::assets::IconName::Folder),
                    (true, "SSH 远端", gpui_kit::assets::IconName::Server),
                ] {
                    modes = modes.child(
                        Button::new(SharedString::from(format!("workspace-mode-{is_remote}")))
                            .icon(glyph)
                            .label(label)
                            .small()
                            .custom(Self::button_style(*remote == is_remote, cx))
                            .on_click(cx.listener(move |s, _, _, cx| {
                                if let Some(SshModal::Workspace { remote, error, .. }) =
                                    &mut s.ssh.modal
                                {
                                    *remote = is_remote;
                                    *error = None;
                                }
                                cx.notify();
                            })),
                    );
                }
                body = body.child(modes);
                if !remote {
                    body = body.child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child("选择本机已有的项目目录，在该目录下启动终端。"),
                    );
                    primary_action = Self::primary(
                        "pick-local-workspace",
                        "选择本地项目目录",
                        cx,
                        |s, w, cx| {
                            s.ssh.modal = None;
                            s.pick_local_workspace(w, cx);
                        },
                    );
                } else if self.session.data.ssh_hosts.is_empty() {
                    body = body.child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child("请先在设置中添加 SSH 主机。"),
                    );
                    primary_action = Self::primary(
                        "workspace-configure-ssh",
                        "添加远端主机",
                        cx,
                        |s, w, cx| {
                            s.ssh.modal = None;
                            s.open_settings(w, cx);
                            s.settings_tab = SettingsTab::SshHosts;
                            cx.notify();
                        },
                    );
                } else {
                    body = body.child(div().text_xs().text_color(rgb(MUTED)).child("选择远端机器"));
                    let mut hosts = div()
                        .id("workspace-ssh-hosts")
                        .max_h(px(180.0))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_2();
                    for machine in &self.session.data.ssh_hosts {
                        let id = machine.id;
                        let selected = *host == Some(id);
                        hosts = hosts.child(
                            div()
                                .id(SharedString::from(format!("workspace-host-{id}")))
                                .p_3()
                                .flex()
                                .items_center()
                                .gap_2()
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(if selected { ACCENT } else { SURFACE }))
                                .bg(rgb(if selected { 0x2c4035 } else { BG }))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(SURFACE)))
                                .on_click(cx.listener(move |s, _, _, cx| {
                                    if let Some(SshModal::Workspace { host, error, .. }) =
                                        &mut s.ssh.modal
                                    {
                                        *host = Some(id);
                                        *error = None;
                                    }
                                    cx.notify();
                                }))
                                .child(icon("server"))
                                .child(
                                    div()
                                        .flex_1()
                                        .flex()
                                        .flex_col()
                                        .child(machine.name.clone())
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(MUTED))
                                                .child(machine.destination()),
                                        ),
                                )
                                .child(icon(if selected { "circle-check" } else { "circle" })),
                        );
                    }
                    body = body
                        .child(hosts)
                        .child(field("远程项目目录", path))
                        .child(field("工作区名称（可选）", name));
                    primary_action = Self::primary(
                        "connect-remote-workspace",
                        "连接并创建工作区",
                        cx,
                        |s, w, cx| s.connect_remote_workspace(w, cx),
                    );
                }
                if let Some(error) = error {
                    body = body.child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xefb3a9))
                            .child(error.clone()),
                    );
                }
            }
            SshModal::Host(form) => {
                body = body
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("SSH 主机"),
                    )
                    .child(field("显示名称", &form.name))
                    .child(field("主机地址 / SSH 别名", &form.host))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(field("用户名（可选）", &form.user)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(field("端口（可选）", &form.port)),
                            ),
                    )
                    .child(field("私钥文件（可选）", &form.key))
                    .child(field("SSH 配置文件（可选）", &form.config));
                if let Some(error) = &form.error {
                    body = body.child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xefb3a9))
                            .child(error.clone()),
                    );
                }
                primary_action = Self::primary(
                    "save-ssh-host",
                    "保存主机",
                    cx,
                    |s, w, cx| s.save_ssh_host(w, cx),
                );
            }
        }
        body = body.child(
            div()
                .id("workspace-modal-actions")
                .flex()
                .items_center()
                .justify_end()
                .gap_3()
                .pt_3()
                .flex_shrink_0()
                .child(
                    Self::button("cancel-ssh-modal", "取消", cx, |s, w, cx| {
                        s.ssh.modal = None;
                        s.focus_current_view(w, cx);
                        cx.notify();
                    })
                    .with_size(gpui_kit::component::Size::Size(px(40.0)))
                    .h(px(40.0))
                    .min_w(px(96.0))
                    .px_3()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD),
                )
                .child(
                    primary_action
                        .with_size(gpui_kit::component::Size::Size(px(40.0)))
                        .h(px(40.0))
                        .min_w(px(160.0))
                        .px_3()
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD),
                ),
        );
        div()
            .absolute()
            .inset_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000099))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(body)
            .into_any_element()
    }
}
