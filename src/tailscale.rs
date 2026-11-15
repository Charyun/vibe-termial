//! Tailscale CLI 定位。
//!
//! 安装位置并不固定在系统盘：安装程序允许改到任意目录（例如 `D:\Program Files\Tailscale`），
//! 因此不能只按 `%ProgramFiles%\Tailscale` 猜测。查找顺序为 PATH、常见安装目录、
//! Windows 安装注册的目录。

use std::path::PathBuf;

/// 平台上的可执行文件名候选，按优先级排列。
pub fn file_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["tailscale.exe"]
    } else {
        &["tailscale", "Tailscale"]
    }
}

/// 在给定目录列表中按顺序查找已存在的可执行文件。
pub fn find_in_path(directories: &[PathBuf], names: &[&str]) -> Option<PathBuf> {
    directories
        .iter()
        .filter(|directory| !directory.as_os_str().is_empty())
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|candidate| candidate.is_file())
}

/// 由安装目录推导候选可执行文件路径，保持顺序并去重。
pub fn candidates(directories: &[PathBuf], names: &[&str]) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for directory in directories {
        if directory.as_os_str().is_empty() {
            continue;
        }
        for name in names {
            let candidate = directory.join(name);
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }
    paths
}

/// 可能包含 Tailscale 的安装目录：环境变量推导的常见位置加安装注册的目录。
pub fn install_directories() -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        for variable in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(variable) {
                directories.push(PathBuf::from(base).join("Tailscale"));
            }
        }
        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            let base = PathBuf::from(base);
            directories.push(base.join("Tailscale"));
            directories.push(base.join("Programs").join("Tailscale"));
        }
        if let Some(directory) = registry_install_directory() {
            directories.push(directory);
        }
    }
    #[cfg(not(windows))]
    {
        for directory in [
            "/usr/local/bin",
            "/opt/homebrew/bin",
            "/usr/bin",
            "/Applications/Tailscale.app/Contents/MacOS",
        ] {
            directories.push(PathBuf::from(directory));
        }
    }
    let mut unique: Vec<PathBuf> = Vec::new();
    for directory in directories {
        if !directory.as_os_str().is_empty() && !unique.contains(&directory) {
            unique.push(directory);
        }
    }
    unique
}

/// 查找本机的 Tailscale CLI；PATH 优先，其次常见安装目录。
pub fn locate() -> Option<PathBuf> {
    let names = file_names();
    let path_directories: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    find_in_path(&path_directories, names).or_else(|| find_in_path(&install_directories(), names))
}

/// 用于执行 `tailscale ip -4` 的命令；都找不到时回退为按 PATH 解析的命令名。
pub fn command() -> std::process::Command {
    match locate() {
        Some(executable) => std::process::Command::new(executable),
        None => std::process::Command::new(file_names()[0]),
    }
}

/// 读取安装程序写入的目录，例如 `D:\Program Files\Tailscale\`。
#[cfg(windows)]
fn registry_install_directory() -> Option<PathBuf> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY_LOCAL_MACHINE, REG_SZ, REG_VALUE_TYPE, RRF_RT_REG_SZ, RegGetValueW,
    };
    let subkey: Vec<u16> = "SOFTWARE\\Tailscale IPN"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut buffer = [0u16; 260];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    let mut kind: REG_VALUE_TYPE = 0;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            std::ptr::null(),
            RRF_RT_REG_SZ,
            &mut kind,
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || kind != REG_SZ {
        return None;
    }
    let limit = (size as usize / std::mem::size_of::<u16>()).min(buffer.len());
    let value = String::from_utf16_lossy(&buffer[..limit.saturating_sub(1)]);
    let value = value.trim().trim_end_matches(['\\', '/']);
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    struct Layout(PathBuf);

    impl Layout {
        fn new(tag: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("vibe-tailscale-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("create fixture root");
            Self(root)
        }

        fn root(&self) -> &Path {
            &self.0
        }

        fn directory(&self, relative: &str) -> PathBuf {
            let path = self.root().join(relative);
            std::fs::create_dir_all(&path).expect("create fixture directory");
            path
        }

        fn file(&self, relative: &str) -> PathBuf {
            let path = self.root().join(relative);
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
    fn find_in_path_skips_missing_directories() {
        let layout = Layout::new("missing");
        let absent = layout.root().join("absent");
        let present = layout.directory("present");
        let expected = layout.file("present/tailscale.exe");
        assert_eq!(
            find_in_path(&[absent, present], &["tailscale.exe"]),
            Some(expected)
        );
    }

    #[test]
    fn find_in_path_ignores_empty_entries() {
        let layout = Layout::new("empty");
        let expected = layout.file("bin/tailscale.exe");
        let directories = [PathBuf::new(), layout.root().join("bin")];
        assert_eq!(
            find_in_path(&directories, &["tailscale.exe"]),
            Some(expected)
        );
    }

    #[test]
    fn find_in_path_prefers_earlier_directory() {
        let layout = Layout::new("order");
        let first = layout.file("first/tailscale.exe");
        layout.file("second/tailscale.exe");
        let directories = [layout.root().join("first"), layout.root().join("second")];
        assert_eq!(find_in_path(&directories, &["tailscale.exe"]), Some(first));
    }

    #[test]
    fn candidates_deduplicate_and_preserve_order() {
        let result = candidates(
            &[
                PathBuf::from("first"),
                PathBuf::from("first"),
                PathBuf::new(),
                PathBuf::from("second"),
            ],
            &["tailscale.exe"],
        );
        assert_eq!(
            result,
            vec![
                PathBuf::from("first").join("tailscale.exe"),
                PathBuf::from("second").join("tailscale.exe"),
            ]
        );
    }

    #[test]
    fn candidates_keep_every_name_for_one_directory() {
        let result = candidates(&[PathBuf::from("bin")], &["tailscale", "Tailscale"]);
        assert_eq!(
            result,
            vec![
                PathBuf::from("bin").join("tailscale"),
                PathBuf::from("bin").join("Tailscale"),
            ]
        );
    }
}
