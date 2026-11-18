use super::{BACK, CELL, FORE, InputCursorStyle, Snapshot, TextCache, prepare};
use gpui_kit::{
    AppContext, Bounds, Context, IntoElement, Render, Window, WindowOptions, div, point, px, size,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use vibe_terminal::terminal::Buffer;

struct Fixture;
impl Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[test]
#[ignore = "opens a native window to verify cursor phases, glyph colors and IME geometry"]
fn native_blink_renders_both_cursor_styles_and_preserves_ime_anchor() {
    let passed = Arc::new(AtomicBool::new(false));
    let finished = passed.clone();
    gpui_kit::application()
        .with_assets(crate::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let root = std::env::temp_dir();
                let mut buffer = Buffer::new(24, 80, root.clone());
                buffer.process(b"x\x08");
                let snapshot = Snapshot::read(&buffer, &root);
                let bounds = Bounds::new(point(px(0.), px(0.)), size(px(800.), px(600.)));
                let anchor = snapshot.ime_bounds(bounds, "", 0).unwrap();
                let mut cache = TextCache::default();
                for style in [InputCursorStyle::Block, InputCursorStyle::Bar] {
                    let shown = prepare(
                        &snapshot, bounds, None, "", style, true, true, &mut cache, window,
                    );
                    let cursor = shown.quads.last().unwrap();
                    assert_eq!(
                        cursor.bounds.size.width,
                        px(if style == InputCursorStyle::Bar {
                            2.
                        } else {
                            CELL
                        })
                    );
                    let ink = if style == InputCursorStyle::Block {
                        BACK
                    } else {
                        FORE
                    };
                    assert!(
                        cache
                            .current
                            .keys()
                            .any(|key| key.text.as_ref() == "x" && key.foreground == ink)
                    );
                    let hidden = prepare(
                        &snapshot, bounds, None, "", style, true, false, &mut cache, window,
                    );
                    assert_eq!(shown.quads.len(), hidden.quads.len() + 1);
                    assert_eq!(shown.lines.len(), hidden.lines.len());
                    assert!(
                        cache
                            .current
                            .keys()
                            .any(|key| key.text.as_ref() == "x" && key.foreground == FORE)
                    );
                    assert_eq!(snapshot.ime_bounds(bounds, "", 0).unwrap(), anchor);
                    // Composition stays painted even during a hidden cursor phase.
                    let composition = prepare(
                        &snapshot, bounds, None, "ni", style, true, false, &mut cache, window,
                    );
                    assert!(composition.lines.len() > hidden.lines.len());
                }
                finished.store(true, Ordering::Release);
                cx.new(|_| Fixture)
            })
            .unwrap();
            cx.quit();
        });
    assert!(passed.load(Ordering::Acquire));
}
