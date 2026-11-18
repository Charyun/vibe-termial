//! Format external file drops as input, never as a submitted command.
use crate::model::{CliKind, CliProfile};
use anyhow::{Result, bail};
use std::path::PathBuf;

pub fn path_input(profile: &CliProfile, paths: &[PathBuf]) -> Result<String> {
    if paths.is_empty() {
        bail!("没有可拖入的文件路径");
    }
    let command = profile.command.replace('\\', "/");
    let executable = command
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let powershell = profile.kind == CliKind::Shell
        && matches!(
            executable.as_str(),
            "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
        );
    let posix_shell = profile.kind == CliKind::Shell
        && !powershell
        && !matches!(executable.as_str(), "cmd" | "cmd.exe");
    let mut quoted = Vec::with_capacity(paths.len());
    for path in paths {
        let Some(text) = path.to_str() else {
            bail!("文件路径包含无法输入的字符");
        };
        // Newlines or terminal control bytes must never become an implicit
        // Enter, escape sequence or bracketed-paste terminator.
        if text.is_empty() || text.chars().any(char::is_control) {
            bail!("文件路径包含换行或控制字符，无法拖入终端");
        }
        let text = if powershell {
            format!("'{}'", text.replace('\'', "''"))
        } else if posix_shell {
            format!("'{}'", text.replace('\'', "'\\''"))
        } else {
            format!("\"{}\"", text.replace('"', "\\\""))
        };
        quoted.push(text);
    }
    // Separate multiple arguments and leave room for the next typed argument.
    Ok(format!("{} ", quoted.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shell(command: &str) -> CliProfile {
        let mut profile = CliProfile::default_shell();
        profile.command = command.into();
        profile
    }
    #[test]
    fn cmd_and_agent_drops_quote_each_unicode_path_without_submitting() {
        let paths = [
            PathBuf::from("C:\\视频 文件\\a.mp4"),
            PathBuf::from("C:\\other.txt"),
        ];
        let expected = "\"C:\\视频 文件\\a.mp4\" \"C:\\other.txt\" ";
        assert_eq!(path_input(&shell("cmd.exe"), &paths).unwrap(), expected);
        let mut agent = shell("codex.cmd");
        agent.kind = CliKind::Codex;
        assert_eq!(path_input(&agent, &paths).unwrap(), expected);
    }
    #[test]
    fn powershell_and_posix_paths_preserve_quotes_and_expansion_characters() {
        let path = PathBuf::from("/tmp/O'Brien $test `file`.txt");
        assert_eq!(
            path_input(
                &shell("C:\\Windows\\powershell.exe"),
                std::slice::from_ref(&path)
            )
            .unwrap(),
            "'/tmp/O''Brien $test `file`.txt' "
        );
        assert_eq!(
            path_input(&shell("/bin/zsh"), &[path]).unwrap(),
            "'/tmp/O'\\''Brien $test `file`.txt' "
        );
    }
    #[test]
    fn control_characters_and_empty_drops_are_rejected_as_a_whole() {
        for path in [
            "file\ncommand",
            "file\rcommand",
            "file\x1b[201~command",
            "file\0name",
        ] {
            assert!(path_input(&shell("cmd"), &["valid.txt".into(), path.into()]).is_err());
        }
        assert!(path_input(&shell("cmd"), &[]).is_err());
    }
}
