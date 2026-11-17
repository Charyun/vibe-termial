//! Cursor animation is independent of terminal snapshots and IME geometry.
use super::*;
use std::time::Instant;

const INTERVAL: Duration = Duration::from_millis(500);

pub(super) struct CursorBlink {
    epoch: Instant,
    enabled: bool,
    visible: bool,
}
impl Default for CursorBlink {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            enabled: false,
            visible: true,
        }
    }
}
impl CursorBlink {
    pub(super) fn reset(&mut self) {
        self.reset_at(Instant::now());
    }
    fn reset_at(&mut self, now: Instant) {
        self.epoch = now;
        self.visible = true;
    }
    fn tick(&mut self, now: Instant, enabled: bool) -> bool {
        let previous = self.visible;
        if self.enabled != enabled {
            self.reset_at(now);
            self.enabled = enabled;
        }
        self.visible = !enabled
            || now.saturating_duration_since(self.epoch).as_millis() / INTERVAL.as_millis() % 2
                == 0;
        self.visible != previous
    }
    pub(super) fn visible(&self) -> bool {
        self.visible
    }
}

impl Ui {
    pub(super) fn update_cursor_blink(&mut self, window: &Window) -> bool {
        let enabled = self.terminal_has_keyboard_focus(window)
            && self.preedit.is_empty()
            && self.snapshot.cursor.is_some()
            && self.session.selected().is_some_and(|id| {
                self.session
                    .terminals
                    .get(&id)
                    .is_some_and(|terminal| terminal.running())
                    && self.agent_activity.get(&id)
                        != Some(&vibe_terminal::activity::Indicator::Working)
            });
        self.cursor_blink.tick(Instant::now(), enabled)
    }
    pub(super) fn spawn_cursor_blink(window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |weak, cx| {
            loop {
                // Only phase changes notify the view. No PTY reads, frame
                // captures or link probes are needed for cursor animation.
                smol::Timer::after(Duration::from_millis(100)).await;
                if weak
                    .update_in(cx, |ui, window, cx| {
                        if ui.update_cursor_blink(window) {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{CursorBlink, Duration, Instant};
    #[test]
    fn idle_cursor_blinks_and_input_restarts_the_visible_phase() {
        let start = Instant::now();
        let mut blink = CursorBlink::default();
        blink.tick(start, true);
        assert!(blink.visible());
        blink.tick(start + Duration::from_millis(499), true);
        assert!(blink.visible());
        assert!(blink.tick(start + Duration::from_millis(500), true));
        assert!(!blink.visible());
        assert!(blink.tick(start + Duration::from_millis(1000), true));
        assert!(blink.visible());
        blink.reset_at(start + Duration::from_millis(1600));
        blink.tick(start + Duration::from_millis(2000), true);
        assert!(blink.visible());
        blink.tick(start + Duration::from_millis(2100), true);
        assert!(!blink.visible());
    }
    #[test]
    fn focus_composition_and_work_suspend_blinking_then_restart_visible() {
        let start = Instant::now();
        let mut blink = CursorBlink::default();
        blink.tick(start, true);
        blink.tick(start + Duration::from_millis(600), true);
        assert!(!blink.visible());
        assert!(blink.tick(start + Duration::from_millis(700), false));
        assert!(blink.visible());
        blink.tick(start + Duration::from_secs(4), false);
        assert!(blink.visible());
        blink.tick(start + Duration::from_secs(5), true);
        assert!(blink.visible());
        blink.tick(start + Duration::from_millis(5500), true);
        assert!(!blink.visible());
    }
}
