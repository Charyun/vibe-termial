use super::*;
use gpui_kit::component::native_menu::NativeMenu;

actions!(terminal_context, [CopyTerminal, PasteTerminal, SelectTerminalScreen]);

impl Ui {
    pub(super) fn set_right_click_menu(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.session.data.right_click_menu = enabled;
        self.session.persist();
        cx.notify();
    }

    pub(super) fn gate_right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button == MouseButton::Right && !self.session.data.right_click_menu {
            cx.stop_propagation();
        }
    }

    pub(super) fn gate_right_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button == MouseButton::Right && !self.session.data.right_click_menu {
            cx.stop_propagation();
        }
    }

    pub(super) fn terminal_context_menu(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.session.data.right_click_menu {
            return;
        }
        self.focus.focus(window, cx);
        let running = self.session.selected()
            .and_then(|id| self.session.terminals.get(&id))
            .is_some_and(|terminal| terminal.running());
        NativeMenu::new()
            .menu_with_disabled("复制", !self.selection.is_some_and(|(a, b)| a != b), Box::new(CopyTerminal))
            .menu_with_disabled("粘贴", !running, Box::new(PasteTerminal))
            .separator()
            .menu("全选当前屏幕", Box::new(SelectTerminalScreen))
            .show(event.position, window, cx);
        cx.stop_propagation();
    }

    pub(super) fn context_copy_terminal(
        &mut self, _: &CopyTerminal, _: &mut Window, cx: &mut Context<Self>,
    ) {
        if self.session.data.right_click_menu {
            self.copy_selection(cx);
        }
        cx.stop_propagation();
    }

    pub(super) fn context_paste_terminal(
        &mut self, _: &PasteTerminal, window: &mut Window, cx: &mut Context<Self>,
    ) {
        if self.session.data.right_click_menu {
            if !self.try_paste_image(window, cx)
                && let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
            {
                self.session.paste(&text);
            }
            self.selection = None;
            cx.notify();
        }
        cx.stop_propagation();
    }

    pub(super) fn context_select_terminal(
        &mut self, _: &SelectTerminalScreen, _: &mut Window, cx: &mut Context<Self>,
    ) {
        if self.session.data.right_click_menu {
            self.selection = Some(((0, 0), (
                self.snapshot.rows.saturating_sub(1) as usize,
                self.snapshot.cols as usize,
            )));
            cx.notify();
        }
        cx.stop_propagation();
    }
}
