#[cfg(windows)]
#[test]
#[ignore = "opens an isolated native SSH host/workspace form; does not connect to external hosts"]
fn native_ssh_host_and_workspace_forms() {
    use super::{SshModal, Ui};
    use gpui_kit::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageW, WM_CHAR};
    let root = std::env::temp_dir().join(format!("vibe-ssh-ui-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let mut session = vibe_terminal::session::Session::new(
        vibe_terminal::store::Store { root: root.clone() },
        Default::default(),
        None,
    );
    session.read_only = true;
    let passed = Arc::new(AtomicBool::new(false));
    let done = passed.clone();
    gpui_kit::application()
        .with_assets(crate::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
            let (handle, view) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1100.0), px(800.0)),
                        cx,
                    ))),
                    ..gpui_kit::component::TitleBar::window_options()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Vibe SSH workspace regression");
                    cx.new(|cx| {
                        let mut ui = Ui::new(session, window, cx);
                        ui.demo = true;
                        ui
                    })
                },
            )
            .unwrap();
            cx.spawn(async move |cx| {
                handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |ui, cx| ui.open_workspace_dialog(window, cx))
                    })
                    .unwrap();
                smol::Timer::after(Duration::from_millis(150)).await;
                handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |ui, cx| {
                            assert!(matches!(
                                ui.ssh.modal,
                                Some(SshModal::Workspace { remote: false, .. })
                            ));
                            assert!(ui.session.data.ssh_hosts.is_empty());
                            ui.ssh.modal = None;
                            ui.settings = true;
                            ui.settings_tab = super::SettingsTab::SshHosts;
                            ui.edit_ssh_host(None, window, cx);
                        })
                    })
                    .unwrap();
                smol::Timer::after(Duration::from_millis(200)).await;
                unsafe {
                    let title: Vec<u16> =
                        "Vibe SSH workspace regression\0".encode_utf16().collect();
                    let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
                    assert!(!hwnd.is_null());
                    for ch in "开发服务器".encode_utf16() {
                        SendMessageW(hwnd, WM_CHAR, ch as usize, 0);
                    }
                }
                handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |ui, cx| {
                            let Some(SshModal::Host(form)) = &ui.ssh.modal else {
                                panic!("missing host form")
                            };
                            assert_eq!(
                                form.name.read(cx).value().as_ref(),
                                "开发服务器",
                                "host name input did not receive native text events"
                            );
                            form.host
                                .update(cx, |field, cx| field.set_value("dev-alias", window, cx));
                            form.user
                                .update(cx, |field, cx| field.set_value("developer", window, cx));
                            form.port
                                .update(cx, |field, cx| field.set_value("2222", window, cx));
                            ui.save_ssh_host(window, cx);
                            assert!(ui.ssh.modal.is_none());
                            assert_eq!(ui.session.data.ssh_hosts.len(), 1);
                            ui.open_workspace_dialog(window, cx);
                            let id = ui.session.data.ssh_hosts[0].id;
                            let Some(SshModal::Workspace {
                                remote,
                                host,
                                path,
                                name,
                                ..
                            }) = &mut ui.ssh.modal
                            else {
                                panic!("missing workspace form")
                            };
                            *remote = true;
                            *host = Some(id);
                            path.update(cx, |field, cx| {
                                field.set_value("/srv/my project", window, cx)
                            });
                            name.update(cx, |field, cx| {
                                field.set_value("Remote project", window, cx)
                            });
                            cx.notify();
                        })
                    })
                    .unwrap();
                smol::Timer::after(Duration::from_millis(200)).await;
                handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |ui, cx| {
                            ui.connect_remote_workspace(window, cx);
                            assert!(ui.ssh.modal.is_none());
                            let workspace = ui.session.data.workspace().unwrap();
                            assert_eq!(workspace.name, "Remote project");
                            assert_eq!(workspace.remote.as_ref().unwrap().path, "/srv/my project");
                            assert_eq!(workspace.tabs[0].profile.name, "SSH Shell");
                            assert!(ui.menu_profiles().iter().all(|p| !p.command.contains(':')));
                            let id = workspace.remote.as_ref().unwrap().host_id;
                            ui.remove_ssh_host(id, cx);
                            assert_eq!(
                                ui.session.data.ssh_hosts.len(),
                                1,
                                "in-use host must not be removed"
                            );
                            assert!(ui.ssh.notice.is_some());
                        })
                    })
                    .unwrap();
                smol::Timer::after(Duration::from_millis(200)).await;
                done.store(true, Ordering::Release);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    assert!(passed.load(Ordering::Acquire));
    let _ = std::fs::remove_dir_all(root);
}
