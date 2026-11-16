//! SSH host configuration and POSIX remote project launch commands.
use crate::model::{CliKind, CliProfile, Tab};
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SshHost {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub identity_file: Option<PathBuf>,
    #[serde(default)]
    pub config_file: Option<PathBuf>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshProject {
    pub host_id: Uuid,
    pub path: String,
}
impl SshHost {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.name.trim().is_empty(), "请输入主机名称");
        ensure!(
            !self.host.is_empty()
                && !self.host.starts_with('-')
                && !self
                    .host
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || c == '@'),
            "主机填写域名、IP 或 SSH 配置别名，用户名请单独填写"
        );
        ensure!(
            !self.user.starts_with('-')
                && !self
                    .user
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '@' | '/' | '\\')),
            "SSH 用户名无效"
        );
        ensure!(self.port != Some(0), "端口必须为 1–65535");
        for (label, file) in [
            ("私钥", &self.identity_file),
            ("SSH 配置", &self.config_file),
        ] {
            if let Some(path) = file {
                ensure!(path.is_file(), "{label}文件不存在：{}", path.display());
            }
        }
        Ok(())
    }
    pub fn destination(&self) -> String {
        let user = if self.user.is_empty() {
            String::new()
        } else {
            format!("{}@", self.user)
        };
        format!(
            "{user}{}{}",
            self.host,
            self.port.map(|p| format!(":{p}")).unwrap_or_default()
        )
    }
    pub fn args(&self, interactive: bool) -> Result<Vec<String>> {
        self.validate()?;
        let mut args = vec![
            "-o".into(),
            "ConnectTimeout=15".into(),
            "-o".into(),
            "ServerAliveInterval=15".into(),
            "-o".into(),
            "ServerAliveCountMax=3".into(),
        ];
        if let Some(file) = &self.config_file {
            args.extend(["-F".into(), file.to_string_lossy().into_owned()]);
        }
        if let Some(port) = self.port {
            args.extend(["-p".into(), port.to_string()]);
        }
        if !self.user.is_empty() {
            args.extend(["-l".into(), self.user.clone()]);
        }
        if let Some(file) = &self.identity_file {
            args.extend(["-i".into(), file.to_string_lossy().into_owned()]);
        }
        if interactive {
            args.push("-tt".into());
        } else {
            args.extend(["-o".into(), "BatchMode=yes".into()]);
        }
        args.extend(["--".into(), self.host.clone()]);
        Ok(args)
    }
}
pub fn validate_project_path(path: &str) -> Result<()> {
    ensure!(
        path == "~" || path.starts_with("~/") || path.starts_with('/'),
        "远程项目目录请填写绝对路径或 ~/ 开头的路径"
    );
    ensure!(
        path.len() <= 4096 && !path.chars().any(char::is_control),
        "远程项目路径包含无效字符或过长"
    );
    Ok(())
}
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
fn directory_expression(path: &str) -> String {
    if path == "~" {
        "\"$HOME\"".into()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("\"$HOME\"/{}", quote(rest))
    } else {
        quote(path)
    }
}
pub fn launch_command(tab: &Tab, project: &SshProject) -> Result<String> {
    validate_project_path(&project.path)?;
    let prefix = format!(
        "cd {} || exit; export TERM=xterm-256color COLORTERM=truecolor; ",
        directory_expression(&project.path)
    );
    if tab.profile.kind == CliKind::Shell {
        return Ok(format!("{prefix}exec \"${{SHELL:-/bin/sh}}\" -l"));
    }
    // A local executable path cannot be reused on a remote machine.
    ensure!(
        !tab.profile.command.contains(['/', '\\', ':']) && !tab.profile.command.starts_with('-'),
        "远端 Agent 命令必须是在远程 PATH 中的命令名称"
    );
    let mut args = tab.profile.args.clone();
    match tab.profile.kind {
        CliKind::Claude => {
            args.push(
                if tab.launched {
                    "--resume"
                } else {
                    "--session-id"
                }
                .into(),
            );
            args.push(
                tab.session_id
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("远端 Claude 会话 ID 缺失"))?,
            );
        }
        CliKind::Codex if tab.launched => {
            if let Some(id) = &tab.session_id {
                args.extend(["resume".into(), id.clone()]);
            } else {
                return Ok(format!(
                    "{prefix}printf '%s\\n' '远端 Codex 会话 ID 未同步。请运行 codex resume 选择原会话，或使用新会话重新开始。'; exec \"${{SHELL:-/bin/sh}}\" -l"
                ));
            }
        }
        _ => {}
    }
    let mut executable = std::iter::once(tab.profile.command.as_str())
        .chain(args.iter().map(String::as_str))
        .map(quote)
        .collect::<Vec<_>>()
        .join(" ");
    let extra = if tab.profile.kind == CliKind::Pi {
        executable.push_str(&format!(
            " --session \"$HOME/.local/state/vibe-terminal/sessions/{}.jsonl\"",
            tab.id
        ));
        "mkdir -p \"$HOME/.local/state/vibe-terminal/sessions\" || exit; export PI_HARDWARE_CURSOR=1; "
    } else {
        ""
    };
    Ok(format!("{prefix}{extra}exec {executable}"))
}
pub fn profiles() -> Vec<CliProfile> {
    [
        ("ssh-shell", "SSH Shell", "", CliKind::Shell),
        ("claude", "Claude Code", "claude", CliKind::Claude),
        ("codex", "Codex", "codex", CliKind::Codex),
        ("pi", "Pi", "pi", CliKind::Pi),
        ("gemini", "Gemini CLI", "gemini", CliKind::Custom),
    ]
    .into_iter()
    .map(|(id, name, command, kind)| CliProfile {
        id: id.into(),
        name: name.into(),
        command: command.into(),
        args: vec![],
        kind,
    })
    .collect()
}
pub fn probe(host: &SshHost) -> Result<String> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut command = Command::new("ssh");
    command
        .args(host.args(false)?)
        .arg("printf '%s\\n' VIBE_SSH_OK; uname -s")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| anyhow::anyhow!("无法启动 OpenSSH：{e}"))?;
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        if let Some(status) = child.try_wait()? {
            let mut out = String::new();
            let mut err = String::new();
            if let Some(pipe) = child.stdout.take() {
                pipe.take(65536).read_to_string(&mut out)?;
            }
            if let Some(pipe) = child.stderr.take() {
                pipe.take(65536).read_to_string(&mut err)?;
            }
            if status.success() && out.lines().any(|line| line == "VIBE_SSH_OK") {
                return Ok(format!("已连接 · {}", out.lines().last().unwrap_or("SSH")));
            }
            bail!(
                "SSH 测试失败：{}。若需首次主机确认或密码认证，请创建 SSH 工作区后在终端完成。",
                err.trim()
            );
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("SSH 连接测试超时");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_quote_remote_paths_and_preserve_ssh_config_defaults() {
        let host = SshHost {
            id: Uuid::new_v4(),
            name: "Dev".into(),
            host: "dev-alias".into(),
            user: String::new(),
            port: None,
            identity_file: None,
            config_file: None,
        };
        let args = host.args(true).unwrap();
        assert!(!args.iter().any(|a| a == "-p" || a == "-l"));
        assert_eq!(&args[args.len() - 2..], ["--", "dev-alias"]);
        let tab = Tab::new(profiles().remove(0), PathBuf::from("/ignored"));
        let project = SshProject {
            host_id: host.id,
            path: "/tmp/a'b; touch /bad".into(),
        };
        let cmd = launch_command(&tab, &project).unwrap();
        assert!(cmd.starts_with("cd '/tmp/a'\"'\"'b; touch /bad' || exit;"));
        assert!(validate_project_path("relative").is_err());
        let mut bad = host;
        bad.host = "-oProxyCommand=bad".into();
        assert!(bad.args(true).is_err());
    }
}
