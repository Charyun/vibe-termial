//! Draggable side panes. Keep preferred widths separate from viewport constraints.
#[cfg(all(test, windows))]
#[path = "resizing_native_test.rs"]
mod native_test;
use super::*;

pub(super) const DIVIDER: f32 = 4.0;
const MIN_SIDEBAR: f32 = 160.0;
const MIN_RIGHT: f32 = 280.0;
const MIN_CENTER: f32 = 320.0;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ResizePane {
    Sidebar,
    Right,
    TerminalPreview,
}
#[derive(Clone, Copy)]
pub(super) struct PaneResize {
    pane: ResizePane,
    origin: Pixels,
    width: f32,
}

fn valid(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}
fn sidebar_max(viewport: f32, right_open: bool) -> f32 {
    (viewport - DIVIDER - MIN_CENTER - if right_open { MIN_RIGHT + DIVIDER } else { 0. })
        .clamp(MIN_SIDEBAR, 480.)
}
fn right_max(viewport: f32, sidebar: f32) -> f32 {
    (viewport - sidebar - DIVIDER * 2. - MIN_CENTER).clamp(MIN_RIGHT, 800.)
}
fn widths(viewport: f32, left: f32, right: f32, right_open: bool) -> (f32, f32) {
    let left = valid(left, 224.).clamp(MIN_SIDEBAR, sidebar_max(viewport, right_open));
    let right = valid(right, 440.).clamp(MIN_RIGHT, right_max(viewport, left));
    (left, right)
}

impl Ui {
    pub(super) fn sync_pane_widths(&mut self, window: &Window) {
        (self.sidebar_width, self.preview_width) = widths(
            f32::from(window.viewport_size().width),
            self.session.data.sidebar_width,
            self.session.data.right_panel_width,
            self.panel.open && !self.settings,
        );
    }
    pub(super) fn begin_pane_resize(&mut self, pane: ResizePane, x: Pixels) {
        self.selecting = false;
        self.path_press = None;
        self.clear_mouse_hover();
        self.pane_resize = Some(PaneResize {
            pane,
            origin: x,
            width: match pane {
                ResizePane::Sidebar => self.sidebar_width,
                ResizePane::Right => self.preview_width,
                ResizePane::TerminalPreview => self.terminal_preview_width,
            },
        });
    }
    fn resize_panes(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.pane_resize else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.finish_pane_resize();
            cx.notify();
            return;
        }
        let delta = f32::from(event.position.x - drag.origin);
        let viewport = f32::from(window.viewport_size().width);
        match drag.pane {
            ResizePane::Sidebar => {
                self.session.data.sidebar_width = (drag.width + delta).clamp(
                    MIN_SIDEBAR,
                    sidebar_max(viewport, self.panel.open && !self.settings),
                )
            }
            ResizePane::Right => {
                self.session.data.right_panel_width =
                    (drag.width - delta).clamp(MIN_RIGHT, right_max(viewport, self.sidebar_width))
            }
            ResizePane::TerminalPreview => {
                self.terminal_preview_width = (drag.width - delta).clamp(120., 800.)
            }
        }
        self.sync_pane_widths(window);
        cx.notify();
    }
    fn finish_pane_resize(&mut self) {
        if let Some(drag) = self.pane_resize.take()
            && drag.pane != ResizePane::TerminalPreview
        {
            self.session.persist();
        }
    }
    pub(super) fn pane_divider(&self, pane: ResizePane, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(match pane {
                ResizePane::Sidebar => "sidebar-divider",
                ResizePane::Right => "preview-divider",
                ResizePane::TerminalPreview => "terminal-preview-divider",
            })
            .w(px(DIVIDER))
            .h_full()
            .flex_shrink_0()
            .occlude()
            .bg(rgb(SURFACE))
            .hover(|style| style.bg(rgb(ACCENT)))
            .cursor(CursorStyle::ResizeLeftRight)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |ui, event: &MouseDownEvent, _, cx| {
                    ui.begin_pane_resize(pane, event.position.x);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .into_any_element()
    }
    pub(super) fn pane_divider_static(&self, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("settings-nav-divider")
            .w(px(DIVIDER))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(SURFACE))
            .into_any_element()
    }
    /// Capture before child editors consume movement/release. The canvas paints
    /// no content and only adds a cursor hitbox while a drag is active.
    pub(super) fn pane_resize_listener(&self, cx: &Context<Self>) -> AnyElement {
        let owner = cx.entity().downgrade();
        let dragging = self.pane_resize.is_some();
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                if dragging {
                    let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                    window.set_cursor_style(CursorStyle::ResizeLeftRight, &hitbox);
                }
                let moved = owner.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = moved.update(cx, |ui, cx| {
                            if ui.pane_resize.is_some() {
                                ui.resize_panes(event, window, cx);
                                cx.stop_propagation();
                            }
                        });
                    }
                });
                let released = owner.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                        let _ = released.update(cx, |ui, cx| {
                            if ui.pane_resize.is_some() {
                                ui.finish_pane_resize();
                                cx.stop_propagation();
                                cx.notify();
                            }
                        });
                    }
                });
            },
        )
        .absolute()
        .size_full()
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{DIVIDER, MIN_CENTER, widths};
    #[test]
    fn widths_preserve_center_and_restore_preferences_after_window_growth() {
        for viewport in [900., 1100., 1360.] {
            let (left, right) = widths(viewport, 480., 800., true);
            assert!(viewport - left - right - DIVIDER * 2. >= MIN_CENTER);
        }
        assert_eq!(widths(900., 224., 440., true), (224., 348.));
        assert_eq!(widths(1360., 224., 440., true), (224., 440.));
        assert_eq!(widths(1360., f32::NAN, f32::INFINITY, true), (224., 440.));
        assert_eq!(widths(1360., 0., -1., true), (160., 280.));
    }
}
