//! Read-only, on-demand project navigation. Filesystem and Git work runs off the UI thread.
use anyhow::{Context, Result, bail};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
}
pub struct Directory {
    pub entries: Vec<Entry>,
    pub truncated: bool,
}
pub struct Changes {
    pub branch: String,
    pub files: Vec<Change>,
    pub truncated: bool,
}
#[derive(Clone)]
pub struct Change {
    pub path: PathBuf,
    pub relative: String,
    pub index: char,
    pub worktree: char,
}
impl Change {
    pub fn staged(&self) -> bool {
        !matches!(self.index, ' ' | '?')
    }
    pub fn unstaged(&self) -> bool {
        self.worktree != ' '
    }
}
pub struct Diff {
    pub path: PathBuf,
    pub text: String,
    pub truncated: bool,
}

pub fn directory(path: &Path) -> Result<Directory> {
    let mut entries = Vec::new();
    let mut truncated = false;
    for item in std::fs::read_dir(path).context("无法读取项目目录")? {
        let item = item?;
        if item.file_name() == ".git" {
            continue;
        }
        if entries.len() >= 500 {
            truncated = true;
            break;
        }
        let kind = item.file_type()?;
        // Do not follow directory symlinks/junctions recursively.
        entries.push(Entry {
            path: item.path(),
            name: item.file_name().to_string_lossy().into_owned(),
            directory: kind.is_dir() && !kind.is_symlink(),
        });
    }
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Directory { entries, truncated })
}
fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(["-c", "core.quotepath=false"])
        .args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().context("无法运行 Git，请检查是否已安装")?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        bail!("{}", error.trim().chars().take(300).collect::<String>());
    }
    Ok(output.stdout)
}
pub fn changes(root: &Path) -> Result<Changes> {
    let repo = git(root, &["rev-parse", "--show-toplevel"])?;
    let repo = PathBuf::from(String::from_utf8_lossy(&repo).trim());
    let branch = git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|v| String::from_utf8_lossy(&v).trim().to_owned())
        .unwrap_or_else(|_| "未创建提交".into());
    let output = git(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut fields = output.split(|b| *b == 0).filter(|v| !v.is_empty());
    let mut files = Vec::new();
    let mut truncated = false;
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            continue;
        }
        let index = field[0] as char;
        let worktree = field[1] as char;
        let relative = String::from_utf8_lossy(&field[3..]).into_owned();
        if matches!(index, 'R' | 'C') || matches!(worktree, 'R' | 'C') {
            fields.next();
        }
        if files.len() >= 500 {
            truncated = true;
            break;
        }
        files.push(Change {
            path: repo.join(&relative),
            relative,
            index,
            worktree,
        });
    }
    Ok(Changes {
        branch,
        files,
        truncated,
    })
}
pub fn diff(root: &Path, change: &Change, staged: bool) -> Result<Diff> {
    let mut bytes = if change.index == '?' {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(&change.path)?
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        bytes
    } else {
        let repo = git(root, &["rev-parse", "--show-toplevel"])?;
        let repo = PathBuf::from(String::from_utf8_lossy(&repo).trim());
        let mut args = vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--unified=3",
        ];
        if staged {
            args.push("--cached");
        }
        args.extend(["--", &change.relative]);
        git(&repo, &args)?
    };
    let truncated = bytes.len() > 2 * 1024 * 1024;
    bytes.truncate(2 * 1024 * 1024);
    let text = if bytes.contains(&0) {
        "二进制文件无法显示文本差异".into()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok(Diff {
        path: change.path.clone(),
        text,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_tree_and_git_changes_use_real_files() {
        let root = std::env::temp_dir().join(format!("vibe-project-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("中文 file.txt"), "first\n").unwrap();
        git(&root, &["init", "-q"]).unwrap();
        let tree = directory(&root).unwrap();
        assert!(tree.entries[0].directory);
        assert_eq!(tree.entries.len(), 2);
        let state = changes(&root).unwrap();
        let file = state
            .files
            .iter()
            .find(|f| f.relative == "中文 file.txt")
            .unwrap();
        assert_eq!(diff(&root, file, false).unwrap().text, "first\n");
        git(&root, &["add", "--", "中文 file.txt"]).unwrap();
        std::fs::write(root.join("中文 file.txt"), "second\n").unwrap();
        let state = changes(&root).unwrap();
        let file = state
            .files
            .iter()
            .find(|f| f.relative == "中文 file.txt")
            .unwrap();
        assert!(file.staged() && file.unstaged());
        assert!(diff(&root, file, true).unwrap().text.contains("+first"));
        assert!(diff(&root, file, false).unwrap().text.contains("+second"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
