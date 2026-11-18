#[cfg(all(test, windows))]
#[path = "file_drop_native_test.rs"]
mod file_drop_native_test;
#[cfg(all(test, windows))]
#[path = "context_menu_native_test.rs"]
mod context_menu_native_test;
#[cfg(all(test, windows))]
#[path = "clipboard_native_test.rs"]
mod clipboard_native_test;
#[cfg(all(test, windows))]
#[path = "conversation_native_test.rs"]
mod native_test;

use super::*;
use gpui_kit::component::{
    input::{Textarea, TextareaState},
    text::TextView,
};
use vibe_terminal::conversation::{ConversationReader, ConversationSnapshot, PendingMessage};
use vibe_terminal::transcript::{Message, Role};

pub(super) struct ConversationView {
    visible: bool,
    generation: u64,
    input: Entity<TextareaState>,
    _subscription: Subscription,
    scroll: ScrollHandle,
    messages: Arc<Vec<Message>>,
    expanded_tools: std::collections::HashSet<usize>,
    source: Option<PathBuf>,
    pending: Vec<PendingMessage>,
    sending: bool,
    image_pastes: usize,
    draft_epoch: u64,
    send_task: Option<Task<()>>,
    notice: Option<String>,
}

impl Ui {
    pub(super) fn drop_composer_files(
        &mut self,
        id: Uuid,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session.selected() != Some(id)
            || !self.conversations.get(&id).is_some_and(|view| view.visible && !view.sending)
        {
            return;
        }
        let Some(text) = self.dropped_paths_text(paths, cx) else { return };
        if let Some(view) = self.conversations.get_mut(&id) {
            view.input.update(cx, |input, cx| {
                input.focus(window, cx);
                input.replace(text, window, cx);
            });
            view.notice = None;
        }
        cx.notify();
    }
    pub(super) fn composer_image_epoch(&self, id: Uuid, entity: EntityId) -> Option<u64> {
        self.conversations
            .get(&id)
            .filter(|view| view.visible && !view.sending && view.input.entity_id() == entity)
            .map(|view| view.draft_epoch)
    }
    pub(super) fn composer_image_destination_current(
        &self,
        id: Uuid,
        entity: EntityId,
        epoch: u64,
    ) -> bool {
        self.composer_image_epoch(id, entity) == Some(epoch)
    }
    pub(super) fn image_composer_target(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<(EntityId, u64)> {
        let view = self
            .session
            .selected()
            .and_then(|id| self.conversations.get(&id))?;
        (view.visible && !view.sending && view.input.read(cx).focus_handle(cx).is_focused(window))
            .then(|| (view.input.entity_id(), view.draft_epoch))
    }
    pub(super) fn native_image_notice(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if let Some(view) = self.conversations.get_mut(&id) {
            view.notice = Some("可继续输入并发送；切换到终端可查看或移除图片。".into());
        }
        cx.notify();
    }
    pub(super) fn begin_composer_image_paste(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if let Some(view) = self.conversations.get_mut(&id) {
            view.image_pastes += 1;
            view.notice = Some("正在准备图片…".into());
        }
        cx.notify();
    }
    pub(super) fn finish_composer_image_paste(
        &mut self,
        id: Uuid,
        entity: EntityId,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self
            .conversations
            .get_mut(&id)
            .filter(|view| view.input.entity_id() == entity)
        {
            view.image_pastes = view.image_pastes.saturating_sub(1);
        }
        cx.notify();
    }
    pub(super) fn insert_composer_image_path(
        &mut self,
        id: Uuid,
        entity: EntityId,
        epoch: u64,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(view) = self.conversations.get_mut(&id).filter(|view| {
            view.input.entity_id() == entity && view.draft_epoch == epoch && !view.sending
        }) else {
            return false;
        };
        view.input
            .update(cx, |input, cx| input.replace(text, window, cx));
        view.notice = None;
        cx.notify();
        true
    }
    pub(super) fn clipboard_image_error(
        &mut self,
        id: Uuid,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.conversations.get_mut(&id) {
            view.notice = Some(message.clone());
        }
        if self.session.selected() == Some(id) {
            self.session.error = Some(message);
        }
        cx.notify();
    }
    pub(super) fn content_visible(&self) -> bool {
        self.active_document.is_none()
            && self
                .session
                .selected()
                .and_then(|id| self.conversations.get(&id))
                .is_some_and(|view| view.visible)
    }
    pub(super) fn focus_current_view(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_document(window, cx) {
            return;
        }
        if let Some(view) = self
            .session
            .selected()
            .and_then(|id| self.conversations.get(&id))
            .filter(|view| view.visible)
        {
            view.input.update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.focus.focus(window, cx);
        }
    }
    pub(super) fn toggle_transcript(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self
            .session
            .selected()
            .and_then(|id| self.session.data.tab(id))
            .cloned()
        else {
            return;
        };
        if self
            .session
            .selected()
            .is_some_and(|id| self.session.data.is_remote_tab(id))
        {
            return;
        }
        if !matches!(
            tab.profile.kind,
            CliKind::Pi | CliKind::Codex | CliKind::Claude
        ) {
            return;
        }
        let id = tab.id;
        let terminal_draft = self.session.content_draft(id);
        if let Some(view) = self.conversations.get_mut(&id) {
            view.visible = !view.visible;
            if view.visible
                && view.input.read(cx).value().is_empty()
                && let Some(draft) = terminal_draft
            {
                view.input
                    .update(cx, |input, cx| input.set_value(draft, window, cx));
            }
        } else {
            let input = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .default_value(terminal_draft.unwrap_or_default())
                    .auto_grow(3, 8)
                    .submit_on_enter(true)
                    .placeholder("输入消息，继续当前会话…")
            });
            let subscription =
                cx.subscribe_in(
                    &input,
                    window,
                    move |ui, _, event, window, cx| match event {
                        InputEvent::PressEnter { shift: false, .. } => {
                            ui.send_content_message(id, window, cx)
                        }
                        InputEvent::Change => cx.notify(),
                        _ => {}
                    },
                );
            self.conversations.insert(
                id,
                ConversationView {
                    visible: true,
                    generation: tab.conversation_generation,
                    input,
                    _subscription: subscription,
                    scroll: ScrollHandle::new(),
                    messages: Arc::new(vec![]),
                    expanded_tools: Default::default(),
                    source: None,
                    pending: vec![],
                    sending: false,
                    image_pastes: 0,
                    draft_epoch: 0,
                    send_task: None,
                    notice: None,
                },
            );
        }
        self.clear_mouse_hover();
        self.focus_current_view(window, cx);
        cx.notify();
    }
    pub(super) fn spawn_conversation_reader(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |weak, cx| {
            let mut reader = ConversationReader::default();
            loop {
                let Ok(request) = weak.update(cx, |ui, _| {
                    let data = &ui.session.data;
                    ui.conversations.retain(|id, _| data.tab(*id).is_some());
                    if ui.settings || !ui.content_visible() {
                        return None;
                    }
                    ui.session
                        .selected()
                        .and_then(|id| ui.session.data.tab(id))
                        // Shell-launched Agents don't use our managed log paths.
                        .filter(|tab| !ui.session.terminals.get(&tab.id)
                            .is_some_and(|terminal| terminal.detected_agent_pid.is_some()))
                        .cloned()
                        .map(|tab| (tab, ui.session.store.root.clone()))
                }) else {
                    break;
                };
                if let Some((tab, root)) = request {
                    let (next, snapshot) = smol::unblock(move || {
                        let snapshot = reader.refresh(&tab, &root);
                        (reader, snapshot)
                    })
                    .await;
                    reader = next;
                    if let Some(snapshot) = snapshot
                        && weak
                            .update(cx, |ui, cx| ui.accept_conversation(snapshot, cx))
                            .is_err()
                    {
                        break;
                    }
                }
                smol::Timer::after(Duration::from_millis(250)).await;
            }
        })
    }
    fn accept_conversation(&mut self, snapshot: ConversationSnapshot, cx: &mut Context<Self>) {
        let Some(tab) = self.session.data.tab(snapshot.key.tab_id) else {
            return;
        };
        if !snapshot.key.matches(tab) {
            return;
        }
        let Some(view) = self.conversations.get_mut(&snapshot.key.tab_id) else {
            return;
        };
        let follow = view.messages.is_empty()
            || f32::from(view.scroll.offset().y + view.scroll.max_offset().y).abs() < LINE * 2.0;
        if view.generation != snapshot.key.generation
            || view.source != snapshot.source
            || snapshot.messages.len() < view.messages.len()
        {
            view.expanded_tools.clear();
        }
        view.expanded_tools.retain(|index| {
            snapshot
                .messages
                .get(*index)
                .is_some_and(|message| message.role == Role::Tool)
        });
        if view.generation != snapshot.key.generation {
            view.generation = snapshot.key.generation;
            view.draft_epoch = view.draft_epoch.wrapping_add(1);
            view.pending.clear();
            view.send_task = None;
            view.sending = false;
            view.notice = None;
        }
        view.pending
            .retain(|pending| !pending.recorded(&snapshot.messages));
        view.messages = snapshot.messages;
        view.source = snapshot.source;
        if follow {
            view.scroll.scroll_to_bottom();
        }
        cx.notify();
    }
    fn send_content_message(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.selected() != Some(id) || !self.content_visible() {
            return;
        }
        let Some(view) = self
            .conversations
            .get(&id)
            .filter(|view| !view.sending && view.image_pastes == 0)
        else {
            return;
        };
        let text = view.input.read(cx).value().to_string();
        let has_image = self
            .native_images
            .get(&id)
            .is_some_and(|target| self.session.paste_target_current(target));
        if text.trim().is_empty() && !has_image {
            return;
        }
        let ticket = match self.session.begin_message_with_image(id, &text, has_image) {
            Ok(ticket) => ticket,
            Err(error) => {
                self.conversations.get_mut(&id).unwrap().notice =
                    Some(format!("无法发送：{error:#}"));
                cx.notify();
                return;
            }
        };
        let view = self.conversations.get_mut(&id).unwrap();
        view.sending = true;
        view.notice = None;
        let baseline = view.messages.len();
        let task = cx.spawn_in(window, async move |weak, cx| {
            // Agent TUIs can swallow Enter when it shares the paste write.
            smol::Timer::after(Duration::from_millis(150)).await;
            let _ = weak.update_in(cx, |ui, window, cx| {
                let result = ui.session.finish_message(ticket);
                if result.is_ok() {
                    ui.native_images.remove(&id);
                }
                let Some(view) = ui.conversations.get_mut(&id) else {
                    return;
                };
                view.sending = false;
                match result {
                    Ok(()) => {
                        if has_image {
                            view.notice = Some("图片消息已提交".into());
                        } else if !text.trim_start().starts_with('/') {
                            let pending = PendingMessage {
                                text: text.clone(),
                                after: baseline,
                            };
                            if !pending.recorded(&view.messages) {
                                view.pending.push(pending);
                            }
                        } else {
                            view.notice = Some("命令已提交；需要选择或授权时可切换到终端。".into());
                        }
                        if view.input.read(cx).value().as_ref() == text {
                            view.draft_epoch = view.draft_epoch.wrapping_add(1);
                            view.input
                                .update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        view.scroll.scroll_to_bottom();
                    }
                    Err(error) => view.notice = Some(format!("消息未提交：{error:#}")),
                }
                if ui.session.selected() == Some(id) && ui.content_visible() && !ui.settings {
                    ui.focus_current_view(window, cx);
                }
                cx.notify();
            });
        });
        self.conversations.get_mut(&id).unwrap().send_task = Some(task);
        cx.notify();
    }
    fn stop_content_message(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if let Some(view) = self.conversations.get_mut(&id) {
            view.send_task = None;
            view.sending = false;
        }
        let result = self.session.interrupt_message(id);
        if let Some(view) = self.conversations.get_mut(&id) {
            view.notice = Some(match result {
                Ok(()) => "已发送停止请求".into(),
                Err(error) => format!("无法停止：{error:#}"),
            });
        }
        cx.notify();
    }
    pub(super) fn conversation_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some((tab, view)) = self
            .session
            .selected()
            .and_then(|id| self.session.data.tab(id).zip(self.conversations.get(&id)))
        else {
            return div().into_any_element();
        };
        let id = tab.id;
        let running = self
            .session
            .terminals
            .get(&id)
            .is_some_and(|terminal| terminal.running());
        let has_image = self
            .native_images
            .get(&id)
            .is_some_and(|target| self.session.paste_target_current(target));
        let can_send = running
            && !view.sending
            && view.image_pastes == 0
            && (has_image || !view.input.read(cx).value().trim().is_empty());
        let mut list = div()
            .id(SharedString::from(format!("conversation-messages-{id}")))
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .track_scroll(&view.scroll)
            .p_4();
        if view.messages.is_empty() && view.pending.is_empty() {
            list = list.child(div().text_sm().text_color(rgb(MUTED)).child(
                if view.source.is_some() {
                    "这个会话还没有消息，可以在下方开始对话。"
                } else {
                    "可以在下方继续当前会话，记录就绪后会自动显示。"
                },
            ));
        }
        for (index, message) in view.messages.iter().enumerate() {
            if message.role == Role::Tool {
                let expanded = view.expanded_tools.contains(&index);
                let mut tool = div().flex().flex_col().gap_2().min_w_0().child(
                    Button::new(SharedString::from(format!(
                        "conversation-tool-{id}-{index}"
                    )))
                    .icon(if expanded {
                        gpui_kit::assets::IconName::ChevronDown
                    } else {
                        gpui_kit::assets::IconName::ChevronRight
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_align(TextAlign::Left)
                            .text_xs()
                            .child(format!("工具 · {} 行", message.text.lines().count().max(1))),
                    )
                    .small()
                    .w_full()
                    .custom(Self::button_style(false, cx))
                    .accessibility_label(if expanded {
                        "收起工具输出"
                    } else {
                        "展开工具输出"
                    })
                    .tooltip(if expanded {
                        "收起工具输出"
                    } else {
                        "展开工具输出"
                    })
                    .toggled(expanded)
                    .on_click(cx.listener(move |ui, _, _, cx| {
                        if let Some(view) = ui.conversations.get_mut(&id) {
                            if !view.expanded_tools.remove(&index) {
                                view.expanded_tools.insert(index);
                            }
                        }
                        cx.notify();
                    })),
                );
                if expanded {
                    tool = tool.child(div().p_3().rounded_md().bg(rgb(SURFACE)).child(
                        TextView::markdown(
                            SharedString::from(format!("conversation-{id}-{index}")),
                            message.text.clone(),
                        ),
                    ));
                }
                list = list.child(tool);
                continue;
            }
            let (label, color) = match message.role {
                Role::User => ("你", ACCENT),
                Role::Assistant => (tab.profile.name.as_str(), FG),
                Role::Tool => ("工具", MUTED),
                Role::System => ("系统", MUTED),
            };
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(color))
                            .child(label.to_owned()),
                    )
                    .child(TextView::markdown(
                        SharedString::from(format!("conversation-{id}-{index}")),
                        message.text.clone(),
                    )),
            );
        }
        for (index, pending) in view.pending.iter().enumerate() {
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .min_w_0()
                    .child(div().text_xs().text_color(rgb(ACCENT)).child("你 · 已提交"))
                    .child(TextView::markdown(
                        SharedString::from(format!("conversation-pending-{id}-{index}")),
                        pending.text.clone(),
                    )),
            );
        }
        let mut actions = div().flex().items_center().gap_2().child(
            div()
                .flex_1()
                .text_xs()
                .text_color(rgb(MUTED))
                .child("Enter 发送 · Shift + Enter 换行"),
        );
        if running {
            actions = actions
                .child(Self::button(
                    "conversation-stop",
                    "停止",
                    cx,
                    move |ui, _, cx| ui.stop_content_message(id, cx),
                ))
                .child(
                    Button::new("conversation-send")
                        .label(if view.sending {
                            "提交中…"
                        } else {
                            "发送"
                        })
                        .small()
                        .primary()
                        .disabled(!can_send)
                        .on_click(cx.listener(move |ui, _, window, cx| {
                            ui.send_content_message(id, window, cx)
                        })),
                );
        } else {
            actions = actions.child(Self::primary(
                "conversation-resume",
                "恢复会话",
                cx,
                move |ui, window, cx| {
                    ui.session.restore(id);
                    ui.refresh();
                    ui.focus_current_view(window, cx);
                },
            ));
        }
        let paste_owner = cx.entity().downgrade();
        let paste_entity = view.input.entity_id();
        let composer = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(self.surface_background(PANEL))
            .children(has_image.then(|| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("图片已粘贴到 Agent"),
                    )
                    .child(Self::button(
                        "conversation-image-terminal",
                        "查看附件",
                        cx,
                        |ui, window, cx| ui.toggle_transcript(window, cx),
                    ))
            }))
            .children(
                view.notice
                    .as_ref()
                    .map(|notice| div().text_xs().text_color(rgb(MUTED)).child(notice.clone())),
            )
            .child(
                div()
                    .id("conversation-file-drop")
                    .w_full()
                    .on_drop(cx.listener(move |ui, paths: &ExternalPaths, window, cx| {
                        ui.drop_composer_files(id, paths, window, cx);
                    }))
                    .child(Textarea::new(&view.input)
                    .disabled(!running || view.sending)
                    .on_paste(move |clipboard, _, cx| {
                        paste_owner
                            .update(cx, |ui, cx| {
                                let Some(epoch) = ui.composer_image_epoch(id, paste_entity) else {
                                    return false;
                                };
                                ui.paste_clipboard_image(
                                    id,
                                    super::clipboard::PasteDestination::Composer {
                                        entity: paste_entity,
                                        epoch,
                                    },
                                    clipboard,
                                    cx,
                                )
                            })
                            .unwrap_or(false)
                    })),
            )
            .child(actions);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .bg(self.surface_background(PANEL))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .child(tab.display_title().to_owned()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(format!("同一会话 · {} 条消息", view.messages.len())),
                    )
                    .child(Self::button(
                        "conversation-latest",
                        "最新消息",
                        cx,
                        move |ui, _, _| {
                            if let Some(view) = ui.conversations.get(&id) {
                                view.scroll.scroll_to_bottom();
                            }
                        },
                    ))
                    .child(Self::button(
                        "close-transcript",
                        "返回终端",
                        cx,
                        |ui, window, cx| ui.toggle_transcript(window, cx),
                    )),
            )
            .child(list)
            .child(composer)
            .into_any_element()
    }
}
