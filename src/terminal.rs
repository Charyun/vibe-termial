use crate::model::{CliKind, Tab};
use anyhow::{Context, Result, bail};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
};

pub const SCROLLBACK: usize = 2000;

pub struct Callbacks {
    input: Option<SyncSender<Vec<u8>>>,
    pub cwd: PathBuf,
}
impl vt100::Callbacks for Callbacks {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        let reply = match (i1, c, first) {
            (None, 'n', 6) => {
                let (r, c) = screen.cursor_position();
                Some(format!("\x1b[{};{}R", r + 1, c + 1))
            }
            (None, 'n', 5) => Some("\x1b[0n".into()),
            (None, 'c', 0) => Some("\x1b[?1;2c".into()),
            (Some(b'>'), 'c', _) => Some("\x1b[>0;1;0c".into()),
            _ => None,
        };
        if let (Some(tx), Some(reply)) = (&self.input, reply) {
            let _ = tx.try_send(reply.into_bytes());
        }
    }
    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        if params.first().copied() == Some(b"7") {
            if let Some(value) = params.get(1).and_then(|v| std::str::from_utf8(v).ok()) {
                if let Ok(url) = url::Url::parse(value) {
                    if let Ok(path) = url.to_file_path() {
                        self.cwd = path;
                    }
                }
            }
        }
    }
}

pub struct Buffer {
    pub parser: vt100::Parser<Callbacks>,
    pub revision: u64,
    #[cfg(feature = "web")]
    pub output: tokio::sync::broadcast::Sender<crate::web::Output>,
}
impl Buffer {
    pub fn new(rows: u16, cols: u16, cwd: PathBuf) -> Self {
        Self {
            parser: vt100::Parser::new_with_callbacks(
                rows,
                cols,
                SCROLLBACK,
                Callbacks { input: None, cwd },
            ),
            revision: 0,
            #[cfg(feature = "web")]
            output: tokio::sync::broadcast::channel(256).0,
        }
    }
    pub fn process(&mut self, data: &[u8]) {
        self.parser.process(data);
        self.revision += 1;
        #[cfg(feature = "web")]
        let _ = self.output.send(crate::web::Output::Data(data.to_vec()));
    }
    pub fn replay(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.revision += 1;
        // History is display-only. It is not fed to the new CLI process.
    }
    pub fn scroll_lines(&mut self, lines: isize) -> usize {
        let screen = self.parser.screen_mut();
        screen.set_scrollback(screen.scrollback().saturating_add_signed(lines));
        self.revision += 1;
        screen.scrollback()
    }
    pub fn snapshot(&mut self) -> Vec<u8> {
        let screen = self.parser.screen_mut();
        let old_offset = screen.scrollback();
        screen.set_scrollback(SCROLLBACK);
        let available = screen.scrollback();
        let (rows, _) = screen.size();
        let mut bytes = b"\x1bc".to_vec();
        // Export retained rows as complete formatted lines, then restore the active grid and modes.
        let mut history_rows = Vec::new();
        let mut history_bytes = 0;
        for offset in 1..=available {
            screen.set_scrollback(offset);
            if let Some(row) = screen.rows_formatted(0, screen.size().1).next() {
                if history_bytes + row.len() > 2 * 1024 * 1024 {
                    break;
                }
                history_bytes += row.len();
                history_rows.push(row);
            }
        }
        let has_history = !history_rows.is_empty();
        for row in history_rows.into_iter().rev() {
            bytes.extend(row);
            bytes.extend_from_slice(b"\r\n");
        }
        // Scroll the final historical rows into the backlog before replacing the active grid.
        if has_history {
            for _ in 1..rows {
                bytes.extend_from_slice(b"\r\n");
            }
        }
        screen.set_scrollback(0);
        bytes.extend(screen.state_formatted());
        screen.set_scrollback(old_offset);
        bytes
    }
}

pub struct Terminal {
    #[cfg(feature = "web")]
    pub instance: uuid::Uuid,
    pub buffer: Arc<Mutex<Buffer>>,
    pub dirty: Arc<AtomicBool>,
    pub ended: Arc<AtomicBool>,
    input: Option<SyncSender<Vec<u8>>>,
    master: Option<Box<dyn MasterPty + Send>>,
    child: Option<Box<dyn Child + Send + Sync>>,
    was_started: bool,
    reader_thread: Option<thread::JoinHandle<()>>,
    writer_thread: Option<thread::JoinHandle<()>>,
    #[cfg(windows)]
    job: Option<Job>,
}

impl Terminal {
    pub fn dormant(rows: u16, cols: u16, cwd: PathBuf, history: &[u8]) -> Self {
        let mut buffer = Buffer::new(rows, cols, cwd);
        buffer.replay(history);
        Self {
            #[cfg(feature = "web")]
            instance: uuid::Uuid::new_v4(),
            buffer: Arc::new(Mutex::new(buffer)),
            dirty: Arc::new(AtomicBool::new(true)),
            ended: Arc::new(AtomicBool::new(false)),
            input: None,
            master: None,
            child: None,
            was_started: false,
            reader_thread: None,
            writer_thread: None,
            #[cfg(windows)]
            job: None,
        }
    }
    pub fn start(
        tab: &Tab,
        sessions: &Path,
        rows: u16,
        cols: u16,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self> {
        if !tab.cwd.is_dir() {
            bail!("项目目录不存在：{}", tab.cwd.display());
        }
        let mut args = tab.launch_args(sessions)?;
        if tab.profile.kind == CliKind::Codex {
            let callback = vec![
                std::env::current_exe()?.to_string_lossy().into_owned(),
                "--record-codex-session".into(),
                sessions
                    .join(format!("{}.codex.json", tab.id))
                    .to_string_lossy()
                    .into_owned(),
            ];
            args.extend([
                "-c".into(),
                format!("notify={}", serde_json::to_string(&callback)?),
            ]);
        }
        let pair = native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut cmd = command_builder(&tab.profile.command)?;
        cmd.args(&args);
        cmd.cwd(crate::paths::local_path(&tab.cwd));
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // Enable OSC 7 directory updates for the managed default shells without changing user profiles.
        if tab.profile.kind == CliKind::Shell && args.is_empty() {
            #[cfg(windows)]
            {
                let executable = Path::new(&tab.profile.command)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_ascii_lowercase();
                if matches!(
                    executable.as_str(),
                    "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
                ) {
                    cmd.args(["-NoLogo", "-NoExit", "-Command", "[Console]::InputEncoding = [Text.UTF8Encoding]::new($false); [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); $OutputEncoding = [Console]::OutputEncoding; function global:prompt { $p = (Get-Location).ProviderPath; $u = [System.Uri]::new($p + [IO.Path]::DirectorySeparatorChar); [Console]::Write(([char]27).ToString() + ']7;' + $u.AbsoluteUri + [char]7); 'PS ' + $p + '> ' }"]);
                } else if matches!(executable.as_str(), "cmd" | "cmd.exe") {
                    cmd.args(["/D", "/K", "chcp 65001>nul"]);
                    cmd.env("PROMPT", "$E]7;file:///$P$E\\$P$G");
                }
            }
            #[cfg(unix)]
            cmd.arg("-l");
        }
        let mut child = pair.slave.spawn_command(cmd).with_context(|| {
            format!(
                "无法启动 {}。请先安装 CLI，或在设置中修改命令路径。",
                tab.profile.name
            )
        })?;
        drop(pair.slave);
        #[cfg(windows)]
        let job = match Job::assign(child.as_raw_handle().context("无法取得 CLI 进程句柄")?)
        {
            Ok(job) => Some(job),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.context("无法创建终端进程组"));
            }
        };
        let setup = (|| -> Result<_> {
            Ok((pair.master.try_clone_reader()?, pair.master.take_writer()?))
        })();
        let (mut reader, mut writer) = match setup {
            Ok(x) => x,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(128);
        let mut buffer = Buffer::new(rows, cols, tab.cwd.clone());
        buffer.parser.callbacks_mut().input = Some(tx.clone());
        let shared = Arc::new(Mutex::new(buffer));
        // A new frontend has not consumed a frame yet. The first output chunk must wake it.
        let dirty = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        let write_ended = ended.clone();
        let writer_thread = thread::spawn(move || {
            while let Ok(bytes) = rx.recv() {
                if writer
                    .write_all(&bytes)
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
            write_ended.store(true, Ordering::Release);
        });
        let (shared_read, dirty_read, ended_read) = (shared.clone(), dirty.clone(), ended.clone());
        let reader_thread = thread::spawn(move || {
            let mut bytes = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut b) = shared_read.lock() {
                            b.process(&bytes[..n]);
                        }
                        // The frontend coalesces wakes. Dirty may already be set by a resize,
                        // so it must not suppress the first real output notification.
                        dirty_read.store(true, Ordering::Release);
                        wake();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            ended_read.store(true, Ordering::Release);
            dirty_read.store(true, Ordering::Release);
            wake();
        });
        Ok(Self {
            #[cfg(feature = "web")]
            instance: uuid::Uuid::new_v4(),
            buffer: shared,
            dirty,
            ended,
            input: Some(tx),
            master: Some(pair.master),
            child: Some(child),
            was_started: true,
            reader_thread: Some(reader_thread),
            writer_thread: Some(writer_thread),
            #[cfg(windows)]
            job,
        })
    }
    pub fn running(&self) -> bool {
        self.child.is_some() && !self.ended.load(Ordering::Acquire)
    }
    pub fn loaded(&self) -> bool {
        self.was_started
    }
    pub fn send(&self, bytes: Vec<u8>) -> Result<()> {
        if !self.running() {
            bail!("终端未运行，请恢复会话或新建终端");
        }
        self.input
            .as_ref()
            .context("终端输入已关闭")?
            .try_send(bytes)
            .context("终端输入繁忙，请稍后重试")
    }
    pub fn paste(&self, text: &str) -> Result<()> {
        let bracketed = self
            .buffer
            .lock()
            .map(|b| b.parser.screen().bracketed_paste())
            .unwrap_or(false);
        let text = text.replace('\x1b', "");
        self.send(if bracketed {
            format!("\x1b[200~{text}\x1b[201~").into_bytes()
        } else {
            text.replace('\n', "\r").into_bytes()
        })
    }
    pub fn update_cwd(&self) {
        #[cfg(target_os = "macos")]
        if let Some(pid) = self.child.as_ref().and_then(|c| c.process_id()) {
            unsafe {
                let mut info: libc::proc_vnodepathinfo = std::mem::zeroed();
                let size = std::mem::size_of_val(&info) as i32;
                if libc::proc_pidinfo(
                    pid as i32,
                    libc::PROC_PIDVNODEPATHINFO,
                    0,
                    &mut info as *mut _ as *mut _,
                    size,
                ) == size
                {
                    let path = std::ffi::CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr());
                    use std::os::unix::ffi::OsStrExt;
                    if let Ok(mut b) = self.buffer.lock() {
                        b.parser.callbacks_mut().cwd =
                            PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes()));
                    }
                }
            }
        }
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        let mut b = self
            .buffer
            .lock()
            .map_err(|_| anyhow::anyhow!("终端缓冲区不可用"))?;
        if b.parser.screen().size() == (rows, cols) {
            return Ok(());
        }
        if let Some(m) = &self.master {
            m.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })?;
        }
        b.parser.screen_mut().set_size(rows, cols);
        b.revision += 1;
        #[cfg(feature = "web")]
        let _ = b.output.send(crate::web::Output::Resize(rows, cols));
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }
    pub fn stop(&mut self) {
        self.input.take();
        // Even a poisoned parser lock must release its input sender. This is
        // resource cleanup only; a damaged parser is never reused for output.
        match self.buffer.lock() {
            Ok(mut b) => b.parser.callbacks_mut().input = None,
            Err(poisoned) => poisoned.into_inner().parser.callbacks_mut().input = None,
        }
        #[cfg(windows)]
        self.job.take(); // KILL_ON_JOB_CLOSE terminates the entire descendant tree.
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            if let Some(pid) = child.process_id() {
                #[cfg(target_os = "macos")]
                kill_macos_descendants(pid as i32);
                // portable-pty creates a session/process group for the terminal child.
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        self.master.take();
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
        self.ended.store(true, Ordering::Release);
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "macos")]
fn kill_macos_descendants(pid: i32) {
    // Interactive shells can put jobs in different process groups. Walk their
    // descendants too so quitting the window does not leave those jobs running.
    fn children(pid: i32, depth: usize) {
        if depth > 64 {
            return;
        }
        let mut ids = vec![0i32; 4096];
        let bytes = unsafe {
            libc::proc_listchildpids(
                pid,
                ids.as_mut_ptr().cast(),
                (ids.len() * std::mem::size_of::<i32>()) as i32,
            )
        };
        if bytes <= 0 {
            return;
        }
        for &child in ids
            .iter()
            .take(bytes as usize / std::mem::size_of::<i32>())
            .filter(|&&p| p > 0)
        {
            unsafe {
                libc::kill(child, libc::SIGSTOP);
            }
            children(child, depth + 1);
            unsafe {
                libc::kill(child, libc::SIGKILL);
            }
        }
    }
    unsafe {
        libc::kill(pid, libc::SIGSTOP);
    }
    children(pid, 0);
}

/// 解析 `.cmd`/`.bat` 启动脚本里声明的 JS 入口。
///
/// cmd-shim 生成的写法是 `%dp0%\...`，Pi 等安装脚本生成的是 `%~dp0...`，两种都要识别。
#[cfg(windows)]
fn shim_entry(contents: &str) -> Option<&str> {
    use std::sync::OnceLock;
    static SHIM_ENTRY: OnceLock<regex::Regex> = OnceLock::new();
    let re = SHIM_ENTRY.get_or_init(|| {
        regex::Regex::new(r#"(?i)%~?dp0%?[\\/]?([^"\r\n]*?\.(?:js|cjs|mjs))"#)
            .expect("shim entry regex")
    });
    re.captures(contents)
        .and_then(|captures| captures.get(1))
        .map(|entry| entry.as_str())
}

fn command_builder(command: &str) -> Result<CommandBuilder> {
    #[cfg(windows)]
    {
        let input = PathBuf::from(command);
        let resolved = if input.is_file() {
            Some(input)
        } else {
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .flat_map(|p| {
                    [
                        p.join(format!("{command}.exe")),
                        p.join(command),
                        p.join(format!("{command}.cmd")),
                        p.join(format!("{command}.bat")),
                    ]
                })
                .find(|p| p.is_file() && p.extension().is_some())
        };
        if let Some(path) = resolved {
            let extension = path.extension().unwrap_or_default().to_string_lossy();
            if extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat") {
                // npm 与 cmd-shim 生成的启动脚本会写明 JS 入口；直接交给 Node 执行，
                // 让引号和元字符保持字面量，而不是变成 cmd.exe 的语法。
                let contents = std::fs::read_to_string(&path)?;
                if let Some(entry) = shim_entry(&contents) {
                    let directory = path.parent().context("CLI 路径缺少父目录")?;
                    let entry = directory.join(entry);
                    if entry.is_file() {
                        let local_node = directory.join("node.exe");
                        let mut builder = CommandBuilder::new(if local_node.is_file() {
                            local_node.into_os_string()
                        } else {
                            "node.exe".into()
                        });
                        builder.arg(entry);
                        return Ok(builder);
                    }
                }
                bail!(
                    "该脚本不是可识别的 npm 启动器，请在 CLI 设置中配置 exe，或配置 node.exe 并将 JS 入口放入 args：{}",
                    path.display()
                );
            }
            return Ok(CommandBuilder::new(path));
        }
    }
    Ok(CommandBuilder::new(command))
}

#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Job {
    fn assign(process: std::os::windows::io::RawHandle) -> Result<Self> {
        use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, process.cast()) == 0
            {
                let e = std::io::Error::last_os_error();
                CloseHandle(handle);
                return Err(e.into());
            }
            Ok(Self(handle))
        }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_preserves_utf8_chunks_color_and_alternate_screen() {
        let mut b = Buffer::new(8, 40, PathBuf::from("."));
        let bytes = "\x1b[31m你好\x1b[0m".as_bytes();
        for part in bytes.chunks(2) {
            b.process(part);
        }
        assert!(b.parser.screen().contents().contains("你好"));
        assert_eq!(
            b.parser.screen().cell(0, 0).unwrap().fgcolor(),
            vt100::Color::Idx(1)
        );
        b.process(b"\x1b[?1049hother screen\x1b[?1049l");
        assert!(b.parser.screen().contents().contains("你好"));
    }
    #[test]
    fn shrinking_a_wide_character_at_the_right_edge_does_not_break_output() {
        let mut b = Buffer::new(3, 10, PathBuf::from("."));
        b.process("\x1b[1;9H中".as_bytes());
        b.parser.screen_mut().set_size(3, 9);
        b.process(b"\x1b[1;9Hx");
        assert!(b.parser.screen().contents().contains('x'));
    }

    #[test]
    fn scrolling_history_survives_incoming_output_and_returns_to_live_screen() {
        let mut b = Buffer::new(4, 30, PathBuf::from("."));
        for row in 0..50 {
            b.process(format!("line-{row:02}\r\n").as_bytes());
        }
        let live = b.parser.screen().contents();
        assert!(b.scroll_lines(10) > 0);
        let older = b.parser.screen().contents();
        assert_ne!(older, live);
        b.process(b"new output\r\n");
        assert_eq!(b.parser.screen().contents(), older);
        assert_eq!(b.scroll_lines(-10000), 0);
        assert!(b.parser.screen().contents().contains("new output"));
    }

    #[test]
    fn snapshot_retains_scrollback_and_the_live_grid() {
        let mut b = Buffer::new(4, 40, PathBuf::from("."));
        for n in 0..20 {
            b.process(format!("line-{n:02}\r\n").as_bytes());
        }
        let before = b.parser.screen().contents();
        let bytes = b.snapshot();
        let restored = Terminal::dormant(4, 40, PathBuf::from("."), &bytes);
        let mut r = restored.buffer.lock().unwrap();
        assert_eq!(r.parser.screen().contents(), before);
        r.parser.screen_mut().set_scrollback(SCROLLBACK);
        assert!(r.parser.screen().contents().contains("line-00"));
    }
    #[test]
    fn snapshot_is_complete_and_replay_does_not_launch_a_process() {
        let mut b = Buffer::new(5, 20, PathBuf::from("."));
        b.process(b"hello\r\nworld");
        let bytes = b.snapshot();
        let term = Terminal::dormant(5, 20, PathBuf::from("."), &bytes);
        assert!(!term.loaded());
        assert!(
            term.buffer
                .lock()
                .unwrap()
                .parser
                .screen()
                .contents()
                .contains("world")
        );
    }
    #[cfg(windows)]
    #[test]
    fn shim_entry_reads_cmd_shim_and_pi_launcher_styles() {
        let npm = "@ECHO off\r\n@IF EXIST \"%dp0%\\node.exe\" (\r\n  \"%dp0%\\node.exe\" \"%dp0%\\node_modules\\pkg\\bin\\cli.js\" %*\r\n)\r\n";
        assert_eq!(shim_entry(npm), Some(r"node_modules\pkg\bin\cli.js"));
        let pi = "@ECHO off\r\nnode \"%~dp0pi-launcher.js\" %*\r\n";
        assert_eq!(shim_entry(pi), Some("pi-launcher.js"));
        assert_eq!(shim_entry("@ECHO off\r\necho nothing to launch\r\n"), None);
    }
}
