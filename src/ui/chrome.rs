use super::*;

impl Ui {
    pub(super) fn tab_separator() -> AnyElement {
        div()
            .absolute()
            .right_0()
            .top(px(8.0))
            .w(px(1.0))
            .h(px(24.0))
            .bg(rgb(0x303030))
            .into_any_element()
    }
    pub(super) fn title_bar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let brand = div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .child(img("logo.png").size(px(20.0)))
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Vibe Terminal"),
            );
        let mut strip = div()
            .id("tab-strip")
            .flex()
            .h_full()
            .items_center()
            .flex_1()
            .min_w_0()
            .overflow_x_scroll();
        if let Some(workspace) = self.session.data.workspace().filter(|_| !self.settings) {
            for tab in &workspace.tabs {
                let id = tab.id;
                let active = self.active_document.is_none() && workspace.selected_tab == Some(id);
                let hover_group: SharedString = format!("terminal-tab-hover-{id}").into();
                strip = strip.child(
                    div()
                        .id(SharedString::from(format!("terminal-tab-container-{id}")))
                        .test_support()
                        .group(hover_group.clone())
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
                                .id(SharedString::from(format!("tab-{id}")))
                                .flex()
                                .items_center()
                                .gap_2()
                                .h_full()
                                .px_3()
                                .min_w_0()
                                .max_w(px(224.0))
                                .text_xs()
                                .text_color(rgb(if active { FG } else { MUTED }))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(SURFACE)))
                                .on_click(cx.listener(move |s, event: &ClickEvent, w, cx| {
                                    if event.click_count() == 2 {
                                        s.open_dialog(Dialog::RenameTab(id), w, cx);
                                    } else {
                                        s.select_tab(id, w, cx);
                                    }
                                    cx.notify();
                                }))
                                .children(self.agent_activity_indicator(id, "tab"))
                                .child(profile_icon(&tab.profile))
                                .child(div().truncate().child(tab.display_title().to_owned())),
                        )
                        .child(
                            div()
                                .invisible()
                                .group_hover(hover_group, |style| style.visible())
                                .child(
                                    Self::tool_button(
                                        SharedString::from(format!("close-{id}")),
                                        "关闭标签",
                                        gpui_kit::assets::IconName::X,
                                        false,
                                        cx,
                                        move |s, w, cx| s.open_dialog(Dialog::CloseTab(id), w, cx),
                                    )
                                    .with_size(gpui_kit::component::Size::Size(px(18.0)))
                                    .size(px(18.0)),
                                ),
                        ),
                );
            }
            strip = strip.children(self.document_tab_buttons(workspace.id, cx));
            strip = strip.child(
                div()
                    .h_full()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .px_2()
                    .child(
                        Button::new("new-terminal")
                            .with_size(gpui_kit::component::Size::Size(px(24.0)))
                            .size(px(24.0))
                            .p_0()
                            .child(control_icon(gpui_kit::assets::IconName::Plus))
                            .custom(Self::button_style(false, cx))
                            .accessibility_label("新建终端")
                            .tooltip("新建终端")
                            .on_click(cx.listener(|s, event: &ClickEvent, _, cx| {
                                let origin = event.position();
                                s.detected_agents = vibe_terminal::agents::detected();
                                s.new_menu = Some((origin.x.into(), origin.y.into()));
                                cx.notify();
                            })),
                    ),
            );
        }
        // Occlude TitleBar's drag hitbox so native hit testing returns HTCLIENT.
        let mut tools = div()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .items_center()
            .gap_4()
            .px_2()
            .h_full()
            .flex_shrink_0();
        if !self.settings {
            let transcript_ready = !self
                .session
                .selected()
                .is_some_and(|id| self.session.data.is_remote_tab(id))
                && self.active_document.is_none()
                && (self.content_visible()
                    || self
                        .session
                        .selected()
                        .and_then(|id| self.session.data.tab(id))
                        .is_some_and(|tab| {
                            matches!(
                                tab.profile.kind,
                                CliKind::Pi | CliKind::Codex | CliKind::Claude
                            )
                        }));
            if transcript_ready {
                tools = tools.child(
                    Self::tool_button(
                        "toggle-transcript",
                        if self.content_visible() {
                            "切换到终端"
                        } else {
                            "切换到会话"
                        },
                        gpui_kit::assets::IconName::BookOpen,
                        self.content_visible(),
                        cx,
                        |s, w, cx| s.toggle_transcript(w, cx),
                    )
                    .with_size(gpui_kit::component::Size::Size(px(20.0))),
                );
            }
            // The last app control sits immediately before native minimize.
            tools = tools.child(
                Self::tool_button(
                    "toggle-right-panel",
                    "展开或折叠右侧面板",
                    gpui_kit::assets::IconName::PanelRight,
                    self.panel.open,
                    cx,
                    |s, w, cx| {
                        s.panel.open = !s.panel.open;
                        if !s.panel.open {
                            s.focus.focus(w, cx);
                        }
                        s.sync_project_panel();
                        cx.notify();
                    },
                )
                .with_size(gpui_kit::component::Size::Size(px(20.0))),
            );
        }
        let native_controls = if cfg!(target_os = "macos") {
            80.0
        } else {
            105.0
        };
        let contents = div()
            .flex()
            .items_center()
            .h_full()
            .min_w_0()
            .w(px((f32::from(window.viewport_size().width)
                - native_controls)
                .max(200.0)))
            .child(brand)
            .child(strip)
            .child(tools);
        gpui_kit::component::TitleBar::new()
            .h(px(40.0))
            .pl(px(if cfg!(target_os = "macos") { 80.0 } else { 0.0 }))
            .pr_0()
            .bg(rgb(PANEL))
            .border_color(rgb(0x303030))
            .child(contents)
            .into_any_element()
    }
    // Both screens use the same footer geometry so a second click returns
    // without moving the pointer after entering settings.
    pub(super) fn navigation_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings;
        div()
            .id("navigation-footer")
            .h(px(40.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .px_1()
            .child(
                Button::new("workspace-navigation")
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(control_icon(if settings {
                                gpui_kit::assets::IconName::ArrowLeft
                            } else {
                                gpui_kit::assets::IconName::Settings
                            }))
                            .child(div().text_xs().child(if settings {
                                "返回工作区"
                            } else {
                                "设置"
                            })),
                    )
                    .with_size(gpui_kit::component::Size::Size(px(24.0)))
                    .h(px(24.0))
                    .px_2()
                    .custom(Self::button_style(false, cx))
                    .accessibility_label(if settings {
                        "返回工作区"
                    } else {
                        "设置"
                    })
                    .tooltip(if settings {
                        "返回工作区"
                    } else {
                        "设置"
                    })
                    .on_click(cx.listener(|s, _, window, cx| {
                        if s.settings {
                            s.settings = false;
                            s.focus_current_view(window, cx);
                        } else {
                            s.open_settings(window, cx);
                        }
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
    pub(super) fn status_bar(&self) -> AnyElement {
        let tab = self
            .session
            .selected()
            .and_then(|id| self.session.data.tab(id));
        let running = self
            .session
            .selected()
            .and_then(|id| self.session.terminals.get(&id))
            .is_some_and(|t| t.running());
        let path = self
            .session
            .data
            .workspace()
            .map(|w| {
                if let Some(remote) = &w.remote {
                    let name = self
                        .session
                        .data
                        .ssh_hosts
                        .iter()
                        .find(|h| h.id == remote.host_id)
                        .map(|h| h.name.as_str())
                        .unwrap_or("SSH");
                    format!("SSH · {name}:{}", remote.path)
                } else {
                    display_path(&w.directory)
                }
            })
            .unwrap_or_default();
        div()
            .h(px(24.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .bg(rgb(PANEL))
            .border_t_1()
            .border_color(rgb(0x30362b))
            .text_xs()
            .text_color(rgb(MUTED))
            .child(
                div()
                    .size(px(4.0))
                    .rounded_full()
                    .bg(rgb(if running { ACCENT } else { MUTED })),
            )
            .child(
                tab.map(|t| t.profile.name.clone())
                    .unwrap_or_else(|| "Vibe Terminal".into()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family("Geist Mono")
                    .child(path),
            )
            .child("UTF8")
            .into_any_element()
    }
}
