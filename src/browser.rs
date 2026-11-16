//! Native browser child view. All operations stay on GPUI's UI thread.
//! WebView2 construction pumps Windows messages: run it outside GPUI app/window
//! borrows and outside a Browser RefCell borrow, never inside a canvas paint.
use anyhow::Result;
use gpui_kit::{Bounds, Pixels, Window};
use std::path::PathBuf;

pub struct NativeParent {
    #[cfg(windows)]
    handle: raw_window_handle::RawWindowHandle,
}
impl NativeParent {
    pub fn capture(window: &Window) -> Result<Self> {
        #[cfg(windows)]
        {
            use raw_window_handle::HasWindowHandle;
            Ok(Self {
                handle: HasWindowHandle::window_handle(window)?.as_raw(),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = window;
            anyhow::bail!("当前平台尚未接入内嵌浏览器，请使用浏览器打开");
        }
    }
}
#[cfg(windows)]
impl raw_window_handle::HasWindowHandle for NativeParent {
    fn window_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError>
    {
        // Captured on the UI thread. Creation checks that the parent still
        // exists; a closed/replaced preview never displays the resulting view.
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(self.handle) })
    }
}

pub struct Browser {
    #[cfg(windows)]
    view: Option<wry::WebView>,
    pub pending: bool,
    pub failed: bool,
    #[cfg(windows)]
    bounds: Option<Bounds<Pixels>>,
    #[cfg(windows)]
    visible: bool,
}
impl Browser {
    pub fn new() -> Self {
        Self {
            #[cfg(windows)]
            view: None,
            pending: false,
            failed: false,
            #[cfg(windows)]
            bounds: None,
            #[cfg(windows)]
            visible: false,
        }
    }
    pub fn initialized(&self) -> bool {
        #[cfg(windows)]
        {
            self.view.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// Called from a foreground executor task with no App/Window/RefCell borrow.
    #[cfg(windows)]
    pub fn create(
        parent: NativeParent,
        bounds: Bounds<Pixels>,
        url: &str,
        data: PathBuf,
        on_navigation: impl Fn(String) + 'static,
    ) -> Result<Self> {
        use raw_window_handle::RawWindowHandle;
        use wry::{WebContext, WebViewBuilder};
        if let RawWindowHandle::Win32(handle) = parent.handle {
            anyhow::ensure!(
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(handle.hwnd.get() as _)
                } != 0,
                "网页预览的父窗口已关闭"
            );
        }
        let mut context = WebContext::new(Some(data));
        let view = WebViewBuilder::new_with_web_context(&mut context)
            .with_url(url)
            .with_devtools(true)
            .with_visible(false)
            .with_bounds(rect(bounds))
            .with_background_color((31, 31, 31, 255))
            .with_navigation_handler(|url| {
                vibe_terminal::paths::is_web_url(&url) || url == "about:blank"
            })
            .with_on_page_load_handler(move |_, url| on_navigation(url))
            .build_as_child(&parent)?;
        Ok(Self {
            view: Some(view),
            pending: false,
            failed: false,
            bounds: Some(bounds),
            visible: false,
        })
    }
    #[cfg(not(windows))]
    pub fn create(
        _: NativeParent,
        _: Bounds<Pixels>,
        _: &str,
        _: PathBuf,
        _: impl Fn(String) + 'static,
    ) -> Result<Self> {
        anyhow::bail!("当前平台尚未接入内嵌浏览器，请使用浏览器打开");
    }

    pub fn show(&mut self, bounds: Bounds<Pixels>) -> Result<()> {
        #[cfg(windows)]
        if let Some(view) = &self.view {
            if self.bounds != Some(bounds) {
                view.set_bounds(rect(bounds))?;
                self.bounds = Some(bounds);
            }
            if !self.visible {
                view.set_visible(true)?;
                self.visible = true;
            }
        }
        #[cfg(not(windows))]
        let _ = bounds;
        Ok(())
    }
    pub fn hide(&mut self) {
        #[cfg(windows)]
        if self.visible {
            if let Some(view) = &self.view {
                let _ = view.set_visible(false);
            }
            self.visible = false;
        }
    }
    #[cfg(all(test, windows))]
    pub fn native_bounds(&self) -> wry::Rect {
        self.view
            .as_ref()
            .expect("browser initialized")
            .bounds()
            .unwrap()
    }
    pub fn open_devtools(&self) {
        #[cfg(windows)]
        if let Some(view) = &self.view {
            view.open_devtools();
        }
    }
    pub fn command(&mut self, script: &str) -> Result<()> {
        #[cfg(windows)]
        if let Some(view) = &self.view {
            view.evaluate_script(script)?;
        }
        #[cfg(not(windows))]
        let _ = script;
        Ok(())
    }
}
#[cfg(windows)]
fn rect(bounds: Bounds<Pixels>) -> wry::Rect {
    use wry::dpi::{LogicalPosition, LogicalSize};
    wry::Rect {
        position: LogicalPosition::new(
            f32::from(bounds.origin.x) as f64,
            f32::from(bounds.origin.y) as f64,
        )
        .into(),
        size: LogicalSize::new(
            f32::from(bounds.size.width) as f64,
            f32::from(bounds.size.height) as f64,
        )
        .into(),
    }
}
