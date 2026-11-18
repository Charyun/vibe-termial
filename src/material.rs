//! Native window materials, with an opaque fallback on unsupported platforms.
use gpui_kit::{App, Hsla, Window, WindowBackgroundAppearance, rgb};
use vibe_terminal::model::WindowMaterial;

pub fn effective(material: WindowMaterial, build: u32) -> WindowBackgroundAppearance {
    if build < 17763 {
        return WindowBackgroundAppearance::Opaque;
    }
    match material {
        WindowMaterial::Mica if build >= 22621 => WindowBackgroundAppearance::MicaBackdrop,
        WindowMaterial::MicaAlt if build >= 22621 => WindowBackgroundAppearance::MicaAltBackdrop,
        _ => WindowBackgroundAppearance::Blurred,
    }
}

pub fn windows_build() -> u32 {
    #[cfg(windows)]
    {
        static BUILD: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        *BUILD.get_or_init(|| {
            #[repr(C)]
            struct Version {
                size: u32,
                major: u32,
                minor: u32,
                build: u32,
                platform: u32,
                service_pack: [u16; 128],
            }
            #[link(name = "ntdll")]
            unsafe extern "system" {
                fn RtlGetVersion(version: *mut Version) -> i32;
            }
            let mut version: Version = unsafe { std::mem::zeroed() };
            version.size = std::mem::size_of::<Version>() as u32;
            if unsafe { RtlGetVersion(&mut version) } >= 0 {
                version.build
            } else {
                0
            }
        })
    }
    #[cfg(not(windows))]
    {
        0
    }
}

pub fn apply(
    material: WindowMaterial,
    window: &mut Window,
    cx: &mut App,
) -> WindowBackgroundAppearance {
    let appearance = effective(material, windows_build());
    // GPUI's individual setters do not clear the other Windows backdrop API.
    // Clear both first so Mica -> Acrylic -> Mica never leaves stale effects.
    window.set_background_appearance(WindowBackgroundAppearance::Opaque);
    #[cfg(windows)]
    if let Some(hwnd) = hwnd(window) {
        use windows_sys::Win32::Graphics::Dwm::{
            DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
        };
        unsafe {
            let dark = 1i32;
            let none = 1u32; // DWMSBT_NONE
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as _,
                (&dark as *const i32).cast(),
                4,
            );
            if windows_build() >= 22621 {
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_SYSTEMBACKDROP_TYPE as _,
                    (&none as *const u32).cast(),
                    4,
                );
            }
        }
    }
    window.set_background_appearance(appearance);
    // Kit's Root paints its own background before the application. It must
    // also be transparent or it completely covers the native material.
    gpui_kit::component::Theme::update(cx, |theme| {
        theme.background = if appearance == WindowBackgroundAppearance::Opaque {
            rgb(0x181818).into()
        } else {
            Hsla::transparent_black()
        };
    });
    window.refresh();
    appearance
}

#[cfg(windows)]
pub fn hwnd(window: &Window) -> Option<windows_sys::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as _),
        _ => None,
    }
}

/// Only the app root supplies the base tint; nested base layers stay clear.
pub fn base_tint(appearance: WindowBackgroundAppearance) -> Hsla {
    let opacity = match appearance {
        WindowBackgroundAppearance::Opaque => 1.,
        WindowBackgroundAppearance::MicaBackdrop => 0.30,
        WindowBackgroundAppearance::MicaAltBackdrop => 0.22,
        _ => 0.60,
    };
    let mut color: Hsla = rgb(0x181818).into();
    color.a = opacity;
    color
}

pub fn surface(appearance: WindowBackgroundAppearance, color: u32) -> Hsla {
    let mut value: Hsla = rgb(color).into();
    if appearance != WindowBackgroundAppearance::Opaque {
        value.a = match color {
            0x181818 => 0.,
            0x1f1f1f => 0.22,
            _ => 0.85,
        };
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{
        WindowBackgroundAppearance as Appearance, WindowMaterial, base_tint, effective, surface,
    };
    #[test]
    fn material_support_and_tints_have_safe_fallbacks() {
        for material in [
            WindowMaterial::Mica,
            WindowMaterial::MicaAlt,
            WindowMaterial::Acrylic,
        ] {
            assert_eq!(effective(material, 0), Appearance::Opaque);
            assert_eq!(effective(material, 19045), Appearance::Blurred);
        }
        assert_eq!(
            effective(WindowMaterial::Mica, 22621),
            Appearance::MicaBackdrop
        );
        assert_eq!(
            effective(WindowMaterial::MicaAlt, 22621),
            Appearance::MicaAltBackdrop
        );
        assert_eq!(
            effective(WindowMaterial::Acrylic, 22621),
            Appearance::Blurred
        );
        assert_eq!(base_tint(Appearance::Opaque).a, 1.);
        assert_eq!(surface(Appearance::MicaBackdrop, 0x181818).a, 0.);
        assert!(base_tint(Appearance::Blurred).a < 1.);
    }
}
