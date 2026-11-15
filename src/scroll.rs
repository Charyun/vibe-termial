//! Accumulate high-resolution wheel/trackpad deltas without losing small events.
#[derive(Default)]
pub struct WheelAccumulator {
    pixels: f32,
}
impl WheelAccumulator {
    pub fn lines(&mut self, delta: f32, line_height: f32) -> isize {
        if !delta.is_finite() || line_height <= 0.0 {
            return 0;
        }
        self.pixels += delta;
        let lines = (self.pixels / line_height).trunc() as isize;
        self.pixels -= lines as f32 * line_height;
        lines
    }
    pub fn reset(&mut self) {
        self.pixels = 0.0;
    }
}
/// Encode application wheel input instead of mapping it to cursor keys.
///
/// Windows ConPTY 会吞掉全屏 TUI 的 DEC 鼠标模式（1000/1002/1006）与备用屏（1049）序列，
/// 外部解析器因此看不到它们。`forward_when_hidden` 由调用方按当前标签类型给出：
/// 全屏 CLI（Pi、Claude Code 等）在看不到鼠标模式时也应当收到滚轮事件。
pub fn mouse_wheel_sequence(
    screen: &vt100::Screen,
    up: bool,
    row: u16,
    col: u16,
    alt: bool,
    control: bool,
    forward_when_hidden: bool,
) -> Option<Vec<u8>> {
    let hidden_conpty_mouse = forward_when_hidden
        && cfg!(windows)
        && screen.mouse_protocol_mode() == vt100::MouseProtocolMode::None;
    if screen.mouse_protocol_mode() == vt100::MouseProtocolMode::None && !hidden_conpty_mouse {
        return None;
    }
    let encoding = if hidden_conpty_mouse {
        vt100::MouseProtocolEncoding::Sgr
    } else {
        screen.mouse_protocol_encoding()
    };
    let button = 64u16 + u16::from(!up) + 8 * u16::from(alt) + 16 * u16::from(control);
    let row = row.saturating_add(1);
    let col = col.saturating_add(1);
    Some(match encoding {
        vt100::MouseProtocolEncoding::Sgr => format!("\x1b[<{button};{col};{row}M").into_bytes(),
        vt100::MouseProtocolEncoding::Default => vec![
            27,
            b'[',
            b'M',
            (button + 32) as u8,
            (col.min(223) + 32) as u8,
            (row.min(223) + 32) as u8,
        ],
        vt100::MouseProtocolEncoding::Utf8 => {
            let mut out = b"\x1b[M".to_vec();
            for value in [button + 32, col.saturating_add(32), row.saturating_add(32)] {
                let mut bytes = [0; 4];
                out.extend_from_slice(
                    char::from_u32(value as u32)
                        .unwrap_or(' ')
                        .encode_utf8(&mut bytes)
                        .as_bytes(),
                );
            }
            out
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plain_screen_without_mouse_modes_scrolls_locally() {
        let parser = vt100::Parser::new(24, 80, 100);
        assert!(mouse_wheel_sequence(parser.screen(), true, 6, 2, false, false, false).is_none());
    }
    #[cfg(windows)]
    #[test]
    fn hidden_conpty_mouse_modes_still_reach_fullscreen_cli() {
        // Pi 会开启 1000/1002/1006，但 ConPTY 吞掉了这些序列，解析器看不到。
        let parser = vt100::Parser::new(24, 80, 100);
        assert_eq!(
            mouse_wheel_sequence(parser.screen(), true, 6, 2, false, false, true).unwrap(),
            b"\x1b[<64;3;7M"
        );
    }
    #[test]
    fn explicit_mouse_modes_receive_sgr_wheel_events() {
        let mut parser = vt100::Parser::new(24, 80, 100);
        parser.process(b"\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1006h");
        assert_eq!(
            mouse_wheel_sequence(parser.screen(), true, 6, 2, false, false, true).unwrap(),
            b"\x1b[<64;3;7M"
        );
        assert_eq!(
            mouse_wheel_sequence(parser.screen(), false, 6, 2, false, false, true).unwrap(),
            b"\x1b[<65;3;7M"
        );
    }
    #[test]
    fn small_trackpad_deltas_accumulate_in_both_directions() {
        let mut wheel = WheelAccumulator::default();
        for _ in 0..9 {
            assert_eq!(wheel.lines(2.0, 20.0), 0);
        }
        assert_eq!(wheel.lines(2.0, 20.0), 1);
        for _ in 0..9 {
            assert_eq!(wheel.lines(-2.0, 20.0), 0);
        }
        assert_eq!(wheel.lines(-2.0, 20.0), -1);
    }
}
