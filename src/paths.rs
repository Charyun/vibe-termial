use regex::Regex;
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug, PartialEq)]
pub struct FileLink {
    pub start: usize,
    pub end: usize,
    pub path: PathBuf,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

pub fn find_links(text: &str, cwd: &Path, workspace: &Path) -> Vec<FileLink> {
    static TOKENS: OnceLock<Regex> = OnceLock::new();
    let re = TOKENS.get_or_init(|| {
        Regex::new(r#"(?:"[^"\r\n]+"|'[^'\r\n]+'|`[^`\r\n]+`)(?::\d+(?::\d+)?)?|[^\s<>"'`|]+"#)
            .expect("static path regex")
    });
    let mut found = vec![];
    for m in re.find_iter(text) {
        let raw = m.as_str();
        let cleaned =
            raw.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'));
        let cleaned = cleaned.strip_suffix('.').unwrap_or(cleaned);
        let offset = raw.find(cleaned).unwrap_or(0);
        let (base, line, column) = split_location(cleaned);
        let base = base.trim_matches(['"', '\'', '`']);
        if base.is_empty() || base.contains("://") {
            continue;
        }
        let expanded = if let Some(rest) = base.strip_prefix("~/") {
            directories::BaseDirs::new().map(|b| b.home_dir().join(rest))
        } else {
            Some(PathBuf::from(base))
        };
        let Some(input) = expanded else {
            continue;
        };
        let path = if input.is_absolute() {
            input
        } else {
            let candidate = cwd.join(&input);
            if candidate.is_file() {
                candidate
            } else {
                workspace.join(input)
            }
        };
        if !path.is_file() {
            continue;
        }
        let byte_start = m.start() + offset;
        found.push(FileLink {
            start: text[..byte_start].width(),
            end: text[..byte_start + cleaned.len()].width(),
            path,
            line,
            column,
        });
    }
    found
}

fn split_location(token: &str) -> (&str, Option<usize>, Option<usize>) {
    let Some((left, tail)) = token.rsplit_once(':') else {
        return (token, None, None);
    };
    let Ok(last) = tail.parse::<usize>() else {
        return (token, None, None);
    };
    if let Some((base, before)) = left.rsplit_once(':') {
        if let Ok(line) = before.parse::<usize>() {
            return (base, Some(line.max(1)), Some(last.max(1)));
        }
    }
    (left, Some(last.max(1)), None)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
}

pub struct Preview {
    pub path: PathBuf,
    pub content: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    original_bytes: Vec<u8>,
    encoding: TextEncoding,
    crlf: bool,
}
impl Preview {
    pub fn load(link: &FileLink) -> anyhow::Result<Self> {
        use anyhow::{Context, bail};
        use std::io::Read;
        const MAX: u64 = 2 * 1024 * 1024;
        let f = std::fs::File::open(&link.path).context("无法打开文件")?;
        if f.metadata()?.len() > MAX {
            bail!("文件超过 2 MB，请使用外部编辑器打开");
        }
        let mut bytes = vec![];
        f.take(MAX + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX {
            bail!("文件超过 2 MB，请使用外部编辑器打开");
        }
        let encoding = if bytes.starts_with(&[0xff, 0xfe]) {
            TextEncoding::Utf16Le
        } else if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            TextEncoding::Utf8Bom
        } else {
            TextEncoding::Utf8
        };
        let content = if encoding == TextEncoding::Utf16Le {
            if (bytes.len() - 2) % 2 != 0 {
                bail!("UTF16 文件长度无效");
            }
            String::from_utf16(
                &bytes[2..]
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect::<Vec<_>>(),
            )
            .context("UTF16 文件解码失败")?
        } else {
            String::from_utf8(
                bytes[if encoding == TextEncoding::Utf8Bom {
                    3
                } else {
                    0
                }..]
                    .to_vec(),
            )
            .context("暂不支持该文件编码；请使用 UTF8 或 UTF16 LE")?
        };
        if content.contains('\0') {
            bail!("该文件是二进制文件，无法以文本预览");
        }
        let crlf = content.contains("\r\n") && !content.replace("\r\n", "").contains('\n');
        Ok(Self {
            path: link.path.canonicalize()?,
            content,
            line: link.line,
            column: link.column,
            original_bytes: bytes,
            encoding,
            crlf,
        })
    }
    pub fn save(&mut self, text: &str) -> anyhow::Result<()> {
        use anyhow::bail;
        if text.len() > 2 * 1024 * 1024 {
            bail!("文件超过 2 MB，无法保存");
        }
        if std::fs::read(&self.path)? != self.original_bytes {
            bail!("文件已被其他程序修改。为避免覆盖外部改动，请复制当前内容后重新打开文件。");
        }
        let permissions = std::fs::metadata(&self.path)?.permissions();
        if permissions.readonly() {
            bail!("文件为只读，无法保存");
        }
        let serialized = if self.crlf {
            text.replace("\r\n", "\n").replace('\n', "\r\n")
        } else {
            text.to_owned()
        };
        let bytes = match self.encoding {
            TextEncoding::Utf8 => serialized.into_bytes(),
            TextEncoding::Utf8Bom => {
                let mut out = vec![0xef, 0xbb, 0xbf];
                out.extend_from_slice(serialized.as_bytes());
                out
            }
            TextEncoding::Utf16Le => {
                let mut out = vec![0xff, 0xfe];
                for unit in serialized.encode_utf16() {
                    out.extend_from_slice(&unit.to_le_bytes());
                }
                out
            }
        };
        crate::store::atomic_write(&self.path, &bytes)?;
        self.original_bytes = bytes;
        self.content = text.to_owned();
        Ok(())
    }
}

/// 去掉 Windows 的 verbatim 前缀（`\\?\`）。
///
/// 目录选择后的 `canonicalize` 在 Windows 上返回 `\\?\D:\...` 形式，而
/// `CreateProcessW` 的 `lpCurrentDirectory` 不接受该前缀，会让子进程落在错误目录。
pub fn local_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let Some(text) = path.to_str() else {
            return path.to_path_buf();
        };
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\")
            && Path::new(rest).is_absolute()
        {
            return PathBuf::from(rest.to_owned());
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locations_preserve_windows_drive_colon() {
        assert_eq!(
            split_location(r"C:\code\main.rs:12:5"),
            (r"C:\code\main.rs", Some(12), Some(5))
        );
        assert_eq!(
            split_location("src/main.rs:7"),
            ("src/main.rs", Some(7), None)
        );
    }
    #[test]
    fn links_require_files_and_support_quoted_spaces_and_unicode_columns() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("my file.rs"), "test").unwrap();
        let text = "修改 'my file.rs':12:3 missing.rs:2 https://example.org";
        let links = find_links(text, &root, &root);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].start, 5);
        assert_eq!(links[0].line, Some(12));
        assert_eq!(links[0].column, Some(3));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn local_path_strips_verbatim_prefix() {
        assert_eq!(
            local_path(Path::new(r"\\?\D:\Codes\AIWorkSpace\mediatools")),
            PathBuf::from(r"D:\Codes\AIWorkSpace\mediatools")
        );
        assert_eq!(
            local_path(Path::new(r"\\?\UNC\server\share\dir")),
            PathBuf::from(r"\\server\share\dir")
        );
        assert_eq!(
            local_path(Path::new(r"D:\Codes\AIWorkSpace\mediatools")),
            PathBuf::from(r"D:\Codes\AIWorkSpace\mediatools")
        );
        assert_eq!(
            local_path(Path::new(
                r"\\?\Volume{0e5c0a12-0000-0000-0000-100000000000}\dir"
            )),
            PathBuf::from(r"\\?\Volume{0e5c0a12-0000-0000-0000-100000000000}\dir")
        );
    }
}
