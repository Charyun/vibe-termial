use gpui_kit::component::Sizable;
use gpui_kit::*;

pub const CONTROL_ICON_SIZE: f32 = 14.0;

pub fn control_icon(name: gpui_kit::assets::IconName) -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::new(name)
        .with_size(gpui_kit::component::Size::Size(px(CONTROL_ICON_SIZE)))
}
use vibe_terminal::model::{CliKind, CliProfile};

pub fn icon(name: &str) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(CONTROL_ICON_SIZE))
        .flex_shrink_0()
        .text_color(rgb(0x90978f))
}

/// One icon mapping shared by the sidebar, tabs and new-terminal menu.
pub fn profile_icon(profile: &CliProfile) -> AnyElement {
    profile_icon_sized(profile, px(16.0))
}

pub fn profile_icon_sized(profile: &CliProfile, icon_size: Pixels) -> AnyElement {
    let asset = match profile.kind {
        CliKind::Claude => Some("agents/claude.svg"),
        CliKind::Codex => Some("agents/codex.svg"),
        CliKind::Pi => Some("agents/pi.svg"),
        CliKind::Custom
            if profile.id.to_ascii_lowercase().contains("gemini")
                || profile.name.to_ascii_lowercase().contains("gemini") =>
        {
            Some("agents/gemini.png")
        }
        _ => None,
    };
    if let Some(asset) = asset {
        img(asset)
            .size(icon_size)
            .flex_shrink_0()
            .object_fit(ObjectFit::Contain)
            .into_any_element()
    } else {
        let color = if profile.command.to_ascii_lowercase().contains("powershell")
            || profile.command.to_ascii_lowercase().contains("pwsh")
        {
            0x9eb8f3
        } else {
            0x929a90
        };
        icon("terminal")
            .size(icon_size)
            .text_color(rgb(color))
            .into_any_element()
    }
}
