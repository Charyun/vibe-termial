//! Small native tab indicators, fed by every PTY and legacy session logs.
#[cfg(all(test, windows))]
#[path = "activity_native_test.rs"]
mod native_test;
use super::*;
use gpui_kit::component::{spinner::Spinner, tooltip::Tooltip};
use vibe_terminal::activity::{ActivityReader, Indicator};
impl Ui {
    pub(super) fn refresh_agent_activity(&mut self) -> bool {
        let next: HashMap<_, _> = self
            .session
            .terminals
            .iter()
            .filter_map(|(id, terminal)| {
                let buffer = terminal.buffer.lock().ok()?;
                Some((
                    *id,
                    buffer
                        .parser
                        .callbacks()
                        .activity
                        .indicator(terminal.running()),
                ))
            })
            .collect();
        if self.agent_activity == next {
            false
        } else {
            self.agent_activity = next;
            true
        }
    }
    pub(super) fn spawn_agent_activity_reader(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |weak, cx| {
            let mut reader = ActivityReader::default();
            loop {
                let Ok((requests, root)) = weak.update(cx, |ui, _| {
                    let requests = ui
                        .session
                        .data
                        .workspaces
                        .iter()
                        .flat_map(|workspace| &workspace.tabs)
                        .filter(|tab| {
                            !ui.demo
                                && !ui.session.data.is_remote_tab(tab.id)
                                && matches!(
                                    tab.profile.kind,
                                    CliKind::Codex | CliKind::Pi | CliKind::Claude
                                )
                        })
                        .filter_map(|tab| {
                            let terminal = ui.session.terminals.get(&tab.id)?;
                            if !terminal.loaded()
                                || terminal
                                    .buffer
                                    .lock()
                                    .ok()?
                                    .parser
                                    .callbacks()
                                    .activity
                                    .native()
                            {
                                return None;
                            }
                            Some((tab.clone(), terminal.buffer.clone()))
                        })
                        .collect::<Vec<_>>();
                    (requests, ui.session.store.root.clone())
                }) else {
                    break;
                };
                if requests.is_empty() {
                    reader = ActivityReader::default();
                } else {
                    let tabs = requests
                        .iter()
                        .map(|(tab, _)| tab.clone())
                        .collect::<Vec<_>>();
                    let runtimes = requests
                        .iter()
                        .filter(|(tab, _)| tab.profile.kind == CliKind::Codex)
                        .filter_map(|(tab, buffer)| Some((tab.id, buffer.lock().ok()?.runtime_id)))
                        .collect::<Vec<_>>();
                    let (next, updates, completed) = smol::unblock(move || {
                        let updates = reader.refresh(&tabs, &root);
                        let completed = reader.completions(&runtimes, &root);
                        (reader, updates, completed)
                    })
                    .await;
                    reader = next;
                    if weak
                        .update(cx, |ui, _| {
                            for id in completed {
                                let Some((source, original)) =
                                    requests.iter().find(|(tab, _)| tab.id == id)
                                else {
                                    continue;
                                };
                                if !ui.session.data.tab(id).is_some_and(|tab| {
                                    tab.conversation_generation == source.conversation_generation
                                }) {
                                    continue;
                                }
                                if let Some(terminal) = ui.session.terminals.get(&id)
                                    && Arc::ptr_eq(original, &terminal.buffer)
                                    && let Ok(mut buffer) = terminal.buffer.lock()
                                {
                                    buffer.parser.callbacks_mut().activity.managed_completed();
                                }
                            }
                            for update in updates {
                                let Some(tab) = ui
                                    .session
                                    .data
                                    .tab(update.key.tab_id)
                                    .filter(|tab| update.key.matches(tab))
                                else {
                                    continue;
                                };
                                let Some(terminal) = ui.session.terminals.get(&tab.id) else {
                                    continue;
                                };
                                let Some((_, original)) =
                                    requests.iter().find(|(source, _)| source.id == tab.id)
                                else {
                                    continue;
                                };
                                if Arc::ptr_eq(original, &terminal.buffer)
                                    && let Ok(mut buffer) = terminal.buffer.lock()
                                {
                                    if update.worked && !update.seed {
                                        buffer.parser.callbacks_mut().activity.fallback(
                                            vibe_terminal::activity::Phase::Working,
                                            false,
                                        );
                                    }
                                    buffer
                                        .parser
                                        .callbacks_mut()
                                        .activity
                                        .fallback(update.phase, update.seed);
                                }
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                if weak
                    .update(cx, |ui, cx| {
                        if ui.refresh_agent_activity() {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
                smol::Timer::after(Duration::from_millis(250)).await;
            }
        })
    }
    pub(super) fn tab_activity_indicator(&self, id: Uuid) -> Option<AnyElement> {
        let indicator = self.agent_activity.get(&id).copied().unwrap_or_default();
        if indicator == Indicator::None {
            return None;
        }
        let label = match indicator {
            Indicator::None => "等待输入",
            Indicator::Working => "正在处理",
            Indicator::Blocked => "等待确认或输入",
            Indicator::Completed => "已完成 · 点击标签清除提示",
            Indicator::Failed => "任务出错 · 点击标签清除提示",
        };
        let mut slot = div()
            .id(SharedString::from(format!("agent-status-{id}")))
            .size(px(14.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx));
        match indicator {
            Indicator::Working => {
                slot = slot.child(
                    Spinner::new()
                        .with_size(gpui_kit::component::Size::Size(px(14.0)))
                        .color(rgb(ACCENT).into()),
                )
            }
            Indicator::Completed | Indicator::Failed | Indicator::Blocked => {
                slot = slot.child(div().size(px(6.0)).rounded_full().bg(rgb(match indicator {
                    Indicator::Completed => 0x70cf94,
                    Indicator::Blocked => 0xdcb365,
                    _ => 0xe68181,
                })))
            }
            Indicator::None => {}
        }
        Some(slot.into_any_element())
    }
}
