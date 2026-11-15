use crate::model::AppData;
use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn open() -> Result<Self> {
        let root = if let Some(path) = std::env::var_os("VIBE_TERMINAL_DATA_DIR") {
            PathBuf::from(path)
        } else {
            ProjectDirs::from("dev", "vibe", "VibeTerminal")
                .context("无法定位用户数据目录")?
                .data_local_dir()
                .into()
        };
        fs::create_dir_all(root.join("history"))?;
        fs::create_dir_all(root.join("sessions"))?;
        Ok(Self { root })
    }
    pub fn load(&self) -> Result<AppData> {
        let path = self.root.join("state.json");
        if !path.exists() {
            return Ok(AppData::default());
        }
        let mut data: AppData = serde_json::from_slice(&fs::read(&path)?)
            .context("工作区记录读取失败；原文件已保留")?;
        if data.version != 1 {
            bail!("不支持的工作区记录版本 {}", data.version);
        }
        // 旧记录可能保存了 `\\?\` 前缀，读取时统一规范化，避免终端启动落在错误目录。
        for workspace in &mut data.workspaces {
            workspace.directory = crate::paths::local_path(&workspace.directory);
            for tab in &mut workspace.tabs {
                tab.cwd = crate::paths::local_path(&tab.cwd);
            }
        }
        // Agent 改为自动检测后，配置里只保留内置终端（CMD / PowerShell）；缺失的补回来。
        data.profiles
            .retain(|profile| profile.kind == crate::model::CliKind::Shell);
        for required in crate::model::CliProfile::defaults() {
            if !data
                .profiles
                .iter()
                .any(|profile| profile.id == required.id)
            {
                data.profiles.push(required);
            }
        }
        Ok(data)
    }
    pub fn save(&self, data: &AppData) -> Result<()> {
        atomic_write(
            &self.root.join("state.json"),
            &serde_json::to_vec_pretty(data)?,
        )
    }
    pub fn save_history(&self, id: Uuid, bytes: &[u8]) -> Result<()> {
        atomic_write(&self.root.join("history").join(format!("{id}.ansi")), bytes)
    }
    pub fn history(&self, id: Uuid) -> Result<Vec<u8>> {
        let path = self.root.join("history").join(format!("{id}.ansi"));
        if !path.exists() {
            return Ok(vec![]);
        }
        // Never read unbounded data into memory after a corrupted or manually edited file.
        use std::io::Read;
        let file = fs::File::open(path)?;
        if file.metadata()?.len() > 16 * 1024 * 1024 {
            bail!("历史记录超过 16 MB，无法加载");
        }
        let mut data = Vec::new();
        file.take(16 * 1024 * 1024).read_to_end(&mut data)?;
        Ok(data)
    }
    pub fn delete_history(&self, id: Uuid) {
        let _ = fs::remove_file(self.root.join("history").join(format!("{id}.ansi")));
    }
}

pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        f.write_all(data)?;
        #[cfg(unix)]
        if let Ok(metadata) = fs::metadata(path) {
            f.set_permissions(metadata.permissions())?;
        }
        f.sync_all()?;
        drop(f);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW,
            };
            let src: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let dst: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let replaced = unsafe {
                if path.exists() {
                    // ReplaceFile preserves the original file's ACL and attributes.
                    ReplaceFileW(
                        dst.as_ptr(),
                        src.as_ptr(),
                        std::ptr::null(),
                        0,
                        std::ptr::null(),
                        std::ptr::null(),
                    )
                } else {
                    MoveFileExW(
                        src.as_ptr(),
                        dst.as_ptr(),
                        MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                    )
                }
            };
            if replaced == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(not(windows))]
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result.with_context(|| format!("无法保存 {}", path.display()))
}
