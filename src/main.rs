#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
extern crate gpui_kit as gpui;
mod appearance;
mod assets;
mod browser;
mod project;
mod render;
mod ui;

use gpui_kit::*;
use std::borrow::Cow;

use anyhow::{Context, Result, bail};
use vibe_terminal::store::{Store, atomic_write};

fn main() {
    if let Err(e) = run() {
        eprintln!("Vibe Terminal: {e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    // Codex invokes this short-lived helper after a completed turn. No window or GPU is created.
    if args.first().is_some_and(|s| s == "--record-codex-session") {
        let path = std::path::PathBuf::from(args.get(1).context("missing session marker path")?);
        let payload = args
            .last()
            .context("missing Codex notification payload")?
            .to_string_lossy();
        if payload.len() > 1024 * 1024 {
            bail!("notification too large");
        }
        let value: serde_json::Value = serde_json::from_str(&payload)?;
        let id = value
            .get("thread-id")
            .or_else(|| value.get("thread_id"))
            .and_then(|v| v.as_str())
            .context("missing thread-id")?;
        uuid::Uuid::parse_str(id)?;
        atomic_write(
            &path,
            &serde_json::to_vec(&serde_json::json!({"thread-id":id}))?,
        )?;
        if args.len() >= 4
            && value.get("type").and_then(|value| value.as_str()) == Some("agent-turn-complete")
        {
            let activity_path = std::path::PathBuf::from(&args[2]);
            atomic_write(
                &activity_path,
                &serde_json::to_vec(&serde_json::json!({"event-id": uuid::Uuid::new_v4()}))?,
            )?;
        }
        return Ok(());
    }
    let store = Store::open()?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(store.root.join("instance.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock).context("已有 Vibe Terminal 实例正在使用该数据目录")?;
    let data = store.load()?;
    let session = vibe_terminal::session::Session::new(store, data, Some(lock));
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_app_identity("dev.vibe.terminal", "Vibe Terminal");
            gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
            gpui_kit::component::Theme::update(cx, |theme| {
                theme.font_family = "Geist".into();
                theme.mono_font_family = "Geist Mono".into();
                theme.secondary_hover = rgb(0x414c45).into();
                theme.secondary_active = rgb(0x587360).into();
            });
            if let Err(e) = cx.text_system().add_fonts(vec![
                Cow::Borrowed(include_bytes!("../assets/fonts/Geist.ttf")),
                Cow::Borrowed(include_bytes!("../assets/fonts/GeistMono.ttf")),
            ]) {
                eprintln!("Font registration failed: {e}");
            }
            let bounds = Bounds::centered(None, size(px(1360.0), px(860.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(900.0), px(560.0))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Vibe Terminal".into()),
                    ..gpui_kit::component::TitleBar::title_bar_options()
                }),
                app_owns_titlebar_drag: true,
                ..Default::default()
            };
            match gpui_kit::open_window(options, cx, move |window, cx| {
                let view = cx.new(|cx| {
                    cx.on_app_quit(|ui: &mut ui::Ui, _| {
                        ui.session.shutdown();
                        async {}
                    })
                    .detach();
                    ui::Ui::new(session, window, cx)
                });
                let weak = view.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    weak.update(cx, |ui, cx| ui.confirm_window_close(window, cx))
                        .unwrap_or(true)
                });
                view
            }) {
                Ok(_) => cx.activate(true),
                Err(e) => {
                    eprintln!("Unable to create window: {e:#}");
                    cx.quit();
                }
            }
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        });
    Ok(())
}
