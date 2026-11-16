use gpui_kit::{AssetSource, SharedString};
use std::borrow::Cow;

/// Product icons and the native toolkit's complete icon catalog, all offline.
pub struct Assets;
const AGENTS: &[(&str, &[u8])] = &[
    ("logo.svg", include_bytes!("../assets/logo.svg")),
    ("logo.png", include_bytes!("../assets/logo.png")),
    (
        "agents/claude.svg",
        include_bytes!("../assets/agents/claude.svg"),
    ),
    (
        "agents/codex.svg",
        include_bytes!("../assets/agents/codex.svg"),
    ),
    ("agents/pi.svg", include_bytes!("../assets/agents/pi.svg")),
    (
        "agents/gemini.png",
        include_bytes!("../assets/agents/gemini.png"),
    ),
];
impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = AGENTS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::AllAssets.load(path)
    }
    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::AllAssets.list(path)?;
        paths.extend(
            AGENTS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| (*name).into()),
        );
        Ok(paths)
    }
}
