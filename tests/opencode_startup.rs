//! 在真实伪终端中验证已安装的 OpenCode 启动器，不提交模型请求。
#![cfg(windows)]

use std::{fs, sync::Arc, time::{Duration, Instant}};
use vibe_terminal::{agents, model::Tab, terminal::Terminal};

#[test]
#[ignore = "需要本机安装 OpenCode；仅执行 --version"]
fn installed_opencode_launcher_runs_in_terminal() {
    let detected = agents::detected().into_iter()
        .find(|agent| agent.spec.id == "opencode").expect("本机应已安装 OpenCode");
    let mut profile = agents::profile(&detected);
    profile.args.push("--version".into());
    let root = std::env::temp_dir().join(format!("vibe-opencode-startup-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let tab = Tab::new(profile, root.clone());
    let result = Terminal::start(&tab, &root, 24, 100, Arc::new(|| {}));
    if result.is_err() {
        fs::remove_dir_all(&root).unwrap();
    }
    let terminal = result.expect("OpenCode 应可通过项目启动器运行");
    let deadline = Instant::now() + Duration::from_secs(15);
    let output = loop {
        let output = terminal.buffer.lock().unwrap().parser.screen().contents();
        if output.contains("opencode v") || Instant::now() >= deadline {
            break output;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(terminal);
    fs::remove_dir_all(root).unwrap();
    assert!(output.contains("opencode v"), "应输出 OpenCode 版本，实际输出：{output}");
}
