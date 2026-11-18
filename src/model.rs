use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CliProfile {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub kind: CliKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliKind {
    Shell,
    Claude,
    Codex,
    Pi,
    Custom,
}

impl CliProfile {
    pub fn default_shell() -> Self {
        let command = if cfg!(windows) {
            "cmd.exe".to_owned()
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
        };
        Self {
            id: "shell".into(),
            name: if cfg!(windows) {
                "CMD".into()
            } else {
                "Shell".into()
            },
            command,
            args: vec![],
            kind: CliKind::Shell,
        }
    }
    /// 内置终端：CMD 与 PowerShell。Agent 由自动检测提供，不再写进配置文件。
    pub fn defaults() -> Vec<Self> {
        vec![
            Self::default_shell(),
            Self {
                id: "powershell".into(),
                name: "PowerShell".into(),
                command: if cfg!(windows) {
                    "powershell.exe".into()
                } else {
                    "pwsh".into()
                },
                args: vec![],
                kind: CliKind::Shell,
            },
        ]
    }
    pub fn parse_list(value: &str) -> Result<Vec<Self>> {
        let profiles: Vec<Self> = serde_json::from_str(value)?;
        if profiles.is_empty()
            || profiles.iter().any(|p| {
                p.id.trim().is_empty() || p.name.trim().is_empty() || p.command.trim().is_empty()
            })
        {
            bail!("至少保留一个 CLI，id、name 和 command 不能为空");
        }
        let ids: std::collections::HashSet<_> = profiles.iter().map(|p| &p.id).collect();
        if ids.len() != profiles.len() {
            bail!("CLI id 不能重复");
        }
        Ok(profiles)
    }
    /// 解析设置页表单里的参数文本：空白分隔，双引号包裹可包含空格，`\"` 表示字面引号。
    pub fn parse_args(input: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        let mut started = false;
        let mut chars = input.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => {
                    quoted = !quoted;
                    started = true;
                }
                '\\' if quoted => {
                    match chars.next() {
                        Some(next) => current.push(next),
                        None => current.push('\\'),
                    }
                    started = true;
                }
                c if c.is_whitespace() && !quoted => {
                    if started {
                        args.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                c => {
                    current.push(c);
                    started = true;
                }
            }
        }
        if started {
            args.push(current);
        }
        args
    }
    /// 把参数数组还原成表单文本，含空白或引号的参数会加引号并转义。
    pub fn format_args(args: &[String]) -> String {
        args.iter()
            .map(|arg| {
                if arg.is_empty() || arg.chars().any(|c| c.is_whitespace() || c == '"') {
                    format!("\"{}\"", arg.replace('"', "\\\""))
                } else {
                    arg.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tab {
    pub id: Uuid,
    pub title: String,
    #[serde(default)]
    pub session_summary: Option<String>,
    #[serde(default)]
    pub title_is_manual: bool,
    #[serde(default)]
    pub conversation_generation: u64,
    pub profile: CliProfile,
    pub cwd: PathBuf,
    pub session_id: Option<String>,
    #[serde(default)]
    pub launched: bool,
    #[serde(default = "default_rows")]
    pub rows: u16,
    #[serde(default = "default_cols")]
    pub cols: u16,
}
fn default_rows() -> u16 {
    30
}
fn default_cols() -> u16 {
    100
}

impl Tab {
    pub fn new(profile: CliProfile, cwd: PathBuf) -> Self {
        let session_id = (profile.kind == CliKind::Claude).then(|| Uuid::new_v4().to_string());
        Self {
            id: Uuid::new_v4(),
            title: profile.name.clone(),
            session_summary: None,
            title_is_manual: false,
            conversation_generation: 0,
            profile,
            cwd,
            session_id,
            launched: false,
            rows: default_rows(),
            cols: default_cols(),
        }
    }

    pub fn accepts_session_summary(&self) -> bool {
        !self.title_is_manual
            && self.profile.kind != CliKind::Shell
            && (self.title == self.profile.name
                || self
                    .title
                    .strip_prefix(&format!("{} ", self.profile.name))
                    .is_some_and(|suffix| suffix.parse::<usize>().is_ok()))
    }

    pub fn display_title(&self) -> &str {
        if self.accepts_session_summary() {
            self.session_summary.as_deref().unwrap_or(&self.title)
        } else {
            &self.title
        }
    }

    pub fn rename(&mut self, title: String) {
        self.title = title;
        self.title_is_manual = true;
    }

    /// Arguments are passed separately, never interpolated into a shell expression.
    pub fn launch_args(&self, session_dir: &Path) -> Result<Vec<String>> {
        let mut args = self.profile.args.clone();
        match self.profile.kind {
            CliKind::Claude => {
                args.push(
                    if self.launched {
                        "--resume"
                    } else {
                        "--session-id"
                    }
                    .into(),
                );
                args.push(
                    self.session_id
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("Claude 会话 ID 缺失"))?,
                );
            }
            CliKind::Codex if self.launched => {
                let Some(id) = &self.session_id else {
                    bail!(
                        "尚未识别到这个 Codex 标签的会话 ID。可保留历史并选择“新会话”，避免恢复到其他标签的对话。"
                    );
                };
                args.extend(["resume".into(), id.clone()]);
            }
            CliKind::Pi => {
                args.extend([
                    "--session".into(),
                    session_dir
                        .join(format!("{}.jsonl", self.id))
                        .to_string_lossy()
                        .into_owned(),
                ]);
            }
            _ => {}
        }
        Ok(args)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub directory: PathBuf,
    pub tabs: Vec<Tab>,
    pub selected_tab: Option<Uuid>,
    #[serde(default)]
    pub right_panel_open: bool,
    #[serde(default)]
    pub remote: Option<crate::ssh::SshProject>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputCursorStyle {
    #[default]
    Block,
    Bar,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMaterial {
    #[default]
    Mica,
    MicaAlt,
    Acrylic,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppData {
    pub version: u32,
    pub workspaces: Vec<Workspace>,
    pub selected_workspace: Option<Uuid>,
    pub profiles: Vec<CliProfile>,
    /// 用户在设置里关掉的 Agent（自动检测结果中不显示、也不进 ＋ 菜单）。
    #[serde(default)]
    pub disabled_agents: Vec<String>,
    #[serde(default)]
    pub input_cursor_style: InputCursorStyle,
    #[serde(default)]
    pub window_material: WindowMaterial,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    #[serde(default = "default_right_panel_width")]
    pub right_panel_width: f32,
    /// 控制终端与文本输入框的右键菜单，旧配置默认关闭。
    #[serde(default)]
    pub right_click_menu: bool,
    #[serde(default)]
    pub ssh_hosts: Vec<crate::ssh::SshHost>,
}

fn default_sidebar_width() -> f32 { 224.0 }
fn default_right_panel_width() -> f32 { 440.0 }

impl Default for AppData {
    fn default() -> Self {
        Self {
            version: 1,
            workspaces: vec![],
            selected_workspace: None,
            profiles: CliProfile::defaults(),
            disabled_agents: vec![],
            input_cursor_style: InputCursorStyle::default(),
            window_material: WindowMaterial::default(),
            sidebar_width: default_sidebar_width(),
            right_panel_width: default_right_panel_width(),
            right_click_menu: false,
            ssh_hosts: vec![],
        }
    }
}

impl AppData {
    pub fn is_remote_tab(&self, id: Uuid) -> bool {
        self.workspaces
            .iter()
            .any(|w| w.remote.is_some() && w.tabs.iter().any(|tab| tab.id == id))
    }
    pub fn remote_for_tab(&self, id: Uuid) -> Option<&crate::ssh::SshProject> {
        self.workspaces
            .iter()
            .find(|w| w.tabs.iter().any(|tab| tab.id == id))
            .and_then(|w| w.remote.as_ref())
    }
    pub fn add_remote_workspace(
        &mut self,
        host_id: Uuid,
        path: String,
        name: String,
    ) -> Result<Uuid> {
        let host = self
            .ssh_hosts
            .iter()
            .find(|h| h.id == host_id)
            .ok_or_else(|| anyhow::anyhow!("SSH 主机已移除，请先在设置中添加主机"))?;
        host.validate()?;
        crate::ssh::validate_project_path(&path)?;
        if let Some(existing) = self.workspaces.iter().find(|w| {
            w.remote
                .as_ref()
                .is_some_and(|r| r.host_id == host_id && r.path == path)
        }) {
            self.selected_workspace = Some(existing.id);
            return Ok(existing.id);
        }
        let id = Uuid::new_v4();
        let name = if name.trim().is_empty() {
            path.trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|n| !n.is_empty() && *n != "~")
                .unwrap_or(&host.name)
                .to_owned()
        } else {
            name.trim().to_owned()
        };
        self.workspaces.push(Workspace {
            id,
            name,
            directory: PathBuf::from(&path),
            tabs: vec![],
            selected_tab: None,
            right_panel_open: false,
            remote: Some(crate::ssh::SshProject { host_id, path }),
        });
        self.selected_workspace = Some(id);
        Ok(id)
    }
    pub fn add_workspace(&mut self, path: &Path) -> Result<Uuid> {
        if !path.is_dir() {
            bail!("请选择存在的本地项目目录");
        }
        let directory = crate::paths::local_path(&path.canonicalize()?);
        if let Some(existing) = self
            .workspaces
            .iter()
            .find(|w| w.remote.is_none() && crate::paths::local_path(&w.directory) == directory)
        {
            self.selected_workspace = Some(existing.id);
            return Ok(existing.id);
        }
        let id = Uuid::new_v4();
        self.workspaces.push(Workspace {
            id,
            name: path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
            directory,
            tabs: vec![],
            selected_tab: None,
            right_panel_open: false,
            remote: None,
        });
        self.selected_workspace = Some(id);
        Ok(id)
    }

    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .find(|w| Some(w.id) == self.selected_workspace)
    }
    pub fn workspace_mut(&mut self) -> Option<&mut Workspace> {
        self.workspaces
            .iter_mut()
            .find(|w| Some(w.id) == self.selected_workspace)
    }
    pub fn tab(&self, id: Uuid) -> Option<&Tab> {
        self.workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .find(|t| t.id == id)
    }
    pub fn tab_mut(&mut self, id: Uuid) -> Option<&mut Tab> {
        self.workspaces
            .iter_mut()
            .flat_map(|w| &mut w.tabs)
            .find(|t| t.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn agent(kind: CliKind, command: &str) -> CliProfile {
        CliProfile {
            id: command.to_owned(),
            name: command.to_owned(),
            command: command.to_owned(),
            args: vec![],
            kind,
        }
    }
    #[test]
    fn profile_lists_reject_duplicate_ids_and_keep_arguments_literal() {
        let profile = CliProfile::default_shell();
        let duplicate = serde_json::to_string(&vec![profile.clone(), profile.clone()]).unwrap();
        assert!(CliProfile::parse_list(&duplicate).is_err());
        let mut custom = profile;
        custom.args = vec!["value with spaces & symbols".into()];
        let parsed =
            CliProfile::parse_list(&serde_json::to_string(&vec![custom]).unwrap()).unwrap();
        assert_eq!(parsed[0].args, ["value with spaces & symbols"]);
        #[cfg(windows)]
        assert_eq!(CliProfile::default_shell().command, "cmd.exe");
    }
    #[test]
    fn resume_is_bound_to_the_tab_and_never_uses_last() {
        let profile = agent(CliKind::Codex, "codex");
        let mut t = Tab::new(profile, PathBuf::from("."));
        assert!(t.launch_args(Path::new("sessions")).unwrap().is_empty());
        t.launched = true;
        assert!(t.launch_args(Path::new("sessions")).is_err());
        t.session_id = Some("known-id".into());
        assert_eq!(
            t.launch_args(Path::new("sessions")).unwrap(),
            ["resume", "known-id"]
        );
    }
    #[test]
    fn claude_keeps_the_same_id_between_new_and_resume() {
        let mut t = Tab::new(agent(CliKind::Claude, "claude"), PathBuf::from("."));
        let initial = t.launch_args(Path::new(".")).unwrap();
        t.launched = true;
        let resumed = t.launch_args(Path::new(".")).unwrap();
        assert_eq!(initial[0], "--session-id");
        assert_eq!(resumed[0], "--resume");
        assert_eq!(initial[1], resumed[1]);
    }
    #[test]
    fn pi_reuses_one_session_file_per_tab() {
        let profile = agent(CliKind::Pi, "pi");
        let mut tab = Tab::new(profile, PathBuf::from("."));
        let sessions = Path::new("sessions");
        let first = tab.launch_args(sessions).unwrap();
        tab.launched = true;
        let resumed = tab.launch_args(sessions).unwrap();
        assert_eq!(first[0], "--session");
        assert_eq!(first, resumed);
        assert_eq!(
            PathBuf::from(&first[1]),
            sessions.join(format!("{}.jsonl", tab.id))
        );
    }
    #[test]
    fn form_args_round_trip_quoted_values() {
        let args = CliProfile::parse_args(r#"--session "/tmp/a b.jsonl" --note "say \"hi\"""#);
        assert_eq!(
            args,
            vec![
                "--session".to_owned(),
                "/tmp/a b.jsonl".to_owned(),
                "--note".to_owned(),
                "say \"hi\"".to_owned(),
            ]
        );
        assert_eq!(
            CliProfile::parse_args(&CliProfile::format_args(&args)),
            args
        );
        assert!(CliProfile::parse_args("   ").is_empty());
    }
}
