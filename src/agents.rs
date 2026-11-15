//! 已安装 Agent CLI 的自动检测。
//!
//! 参考常见 CLI 工具的做法：在 PATH 与若干常见安装目录里按可执行文件名查找，
//! 命中即认为该 Agent 可用。检测结果只用于界面展示与新建菜单，不需要用户手工配置。

use crate::model::{CliKind, CliProfile};
use std::path::{Path, PathBuf};

/// 一个已知 Agent CLI 的静态描述。
pub struct AgentSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub command: &'static str,
    pub kind: CliKind,
}

/// 内置识别的 Agent 列表；新增 Agent 只需在这里加一行。
pub const KNOWN: &[AgentSpec] = &[
    AgentSpec {
        id: "claude",
        name: "Claude Code",
        command: "claude",
        kind: CliKind::Claude,
    },
    AgentSpec {
        id: "codex",
        name: "Codex",
        command: "codex",
        kind: CliKind::Codex,
    },
    AgentSpec {
        id: "pi",
        name: "Pi",
        command: "pi",
        kind: CliKind::Pi,
    },
    AgentSpec {
        id: "gemini",
        name: "Gemini CLI",
        command: "gemini",
        kind: CliKind::Custom,
    },
    AgentSpec {
        id: "opencode",
        name: "OpenCode",
        command: "opencode",
        kind: CliKind::Custom,
    },
    AgentSpec {
        id: "aider",
        name: "Aider",
        command: "aider",
        kind: CliKind::Custom,
    },
    AgentSpec {
        id: "qwen",
        name: "Qwen Code",
        command: "qwen",
        kind: CliKind::Custom,
    },
    AgentSpec {
        id: "cursor-agent",
        name: "Cursor Agent",
        command: "cursor-agent",
        kind: CliKind::Custom,
    },
    AgentSpec {
        id: "amp",
        name: "Amp",
        command: "amp",
        kind: CliKind::Custom,
    },
    AgentSpec {
        id: "copilot",
        name: "GitHub Copilot CLI",
        command: "copilot",
        kind: CliKind::Custom,
    },
    AgentSpec {
        id: "goose",
        name: "Goose",
        command: "goose",
        kind: CliKind::Custom,
    },
    AgentSpec {
        id: "crush",
        name: "Crush",
        command: "crush",
        kind: CliKind::Custom,
    },
];

/// 检测成功的一个 Agent。
#[derive(Clone)]
pub struct Detected {
    pub spec: &'static AgentSpec,
    pub path: PathBuf,
}

/// 可执行文件名后缀，按优先级排列。
fn file_names(command: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            format!("{command}.exe"),
            format!("{command}.cmd"),
            format!("{command}.bat"),
        ]
    } else {
        vec![command.to_owned()]
    }
}

/// 除 PATH 之外，CLI 常见的安装目录。
pub fn extra_directories() -> Vec<PathBuf> {
    fn push_home(directories: &mut Vec<PathBuf>, relative: &str) {
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            directories.push(PathBuf::from(home).join(relative));
        }
    }
    let mut directories = Vec::new();
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            directories.push(PathBuf::from(appdata).join("npm"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            directories.push(PathBuf::from(&local).join("Programs"));
            directories.push(PathBuf::from(local).join("pnpm"));
        }
        for relative in [".pi/agent/bin", ".bun/bin", ".cargo/bin", ".local/bin"] {
            push_home(&mut directories, relative);
        }
    }
    #[cfg(not(windows))]
    {
        for directory in ["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin"] {
            directories.push(PathBuf::from(directory));
        }
        for relative in [".local/bin", ".bun/bin", ".cargo/bin", ".pi/agent/bin"] {
            push_home(&mut directories, relative);
        }
    }
    directories
}

/// 在给定目录里查找可执行文件：目录顺序优先，其次按后缀优先级。
pub fn resolve_in(directories: &[PathBuf], command: &str) -> Option<PathBuf> {
    let names = file_names(command);
    for directory in directories {
        if directory.as_os_str().is_empty() {
            continue;
        }
        for name in &names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn search_directories() -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    directories.extend(extra_directories());
    directories
}

/// 解析单个命令的位置。
pub fn resolve(command: &str) -> Option<PathBuf> {
    let direct = Path::new(command);
    if direct.is_file() {
        return Some(direct.to_path_buf());
    }
    resolve_in(&search_directories(), command)
}

/// 扫描所有已知 Agent，返回本机已安装的部分。
pub fn detected() -> Vec<Detected> {
    let directories = search_directories();
    KNOWN
        .iter()
        .filter_map(|spec| {
            resolve_in(&directories, spec.command).map(|path| Detected { spec, path })
        })
        .collect()
}

/// 把检测结果转换为可直接启动的 CLI 配置。
pub fn profile(detected: &Detected) -> CliProfile {
    CliProfile {
        id: detected.spec.id.to_owned(),
        name: detected.spec.name.to_owned(),
        command: detected.path.to_string_lossy().into_owned(),
        args: Vec::new(),
        kind: detected.spec.kind.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Layout(PathBuf);

    impl Layout {
        fn new(tag: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("vibe-agents-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("create fixture root");
            Self(root)
        }

        fn file(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
            std::fs::write(&path, b"").expect("write fixture file");
            path
        }
    }

    impl Drop for Layout {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn resolve_prefers_earlier_directories_and_known_suffixes() {
        let layout = Layout::new("resolve");
        layout.file("second/claude.exe");
        let expected = layout.file("first/claude.cmd");
        let result = resolve_in(&[layout.0.join("first"), layout.0.join("second")], "claude");
        assert_eq!(result, Some(expected));
    }

    #[test]
    fn resolve_returns_none_for_missing_commands() {
        let layout = Layout::new("missing");
        std::fs::create_dir_all(layout.0.join("empty")).unwrap();
        assert_eq!(
            resolve_in(&[layout.0.join("empty")], "definitely-not-installed"),
            None
        );
    }

    #[test]
    fn detection_only_returns_existing_paths() {
        for detected in detected() {
            assert!(detected.path.is_file(), "{} 路径不存在", detected.spec.name);
            assert!(!detected.spec.id.is_empty());
        }
    }

    #[test]
    fn profile_keeps_detected_command_path() {
        let layout = Layout::new("profile");
        let path = layout.file("bin/pi.exe");
        let spec = KNOWN.iter().find(|spec| spec.id == "pi").unwrap();
        let detected = Detected {
            spec,
            path: path.clone(),
        };
        let profile = profile(&detected);
        assert_eq!(profile.id, "pi");
        assert_eq!(profile.command, path.to_string_lossy());
        assert_eq!(profile.kind, CliKind::Pi);
    }
}
