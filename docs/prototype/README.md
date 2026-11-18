# VibeTerm 交互原型

用浏览器直接打开 [index.html](index.html)。无需构建或启动后端，字体从项目的 `assets/fonts` 本地加载。

本轮参考 [Orca 的终端界面](https://github.com/stablyai/orca)，采用项目与会话树、单层终端标签、终端内输入和按需文件预览。保留当前项目已有功能，不引入 Orca 的 Git、worktree 或编排功能。

## 可体验的操作

- 工作区名称前提供垂直居中的展开／折叠箭头，各项目分别记住折叠状态。列表内只显示已有终端，不放新建入口；顶部 `＋` 选择 CMD、PowerShell 或 Agent。终端名称使用更紧凑的 11px 字号。
- Pi、Codex、Claude 与 Gemini 使用本地图片图标，来源与许可证见 [icons/README.md](icons/README.md)。
- 双击标签重命名；标签上的 `×` 关闭会话。右上角 `···` 可以停止、恢复或重新启动。
- 最顶部标签栏右端的面板按钮可展开或折叠右侧面板。右侧只有“资源管理器”和“源代码控制”两个水平 Tab，没有独立的预览 Tab。折叠和切换 Tab 会保留编辑草稿。
- 点击终端输出中的 `src/ui.rs:12`，会自动展开右侧资源管理器，并在文件树下方打开源码详情；支持编辑、换行、复制、保存和未保存确认，拖动面板分隔线可调整宽度。
- 资源管理器支持目录展开和选择文件。源代码控制使用演示工作树，支持查看差异和暂存／取消暂存，不调用真实 Git。
- 面板类别集中登记在 `panelTabs`，每个类别提供 id、label 与渲染函数，便于后续扩展。
- 点击终端输出中的图片路径查看示例图片，没有独立的图片打开入口。
- 右上角的内容图标切换 Pi / Codex 对话记录。
- 左下角仅保留设置入口，Agent、远程访问和快捷键均在设置内。工作区右侧的 `···` 菜单保留项目概览、重命名与移除。
- 在远程访问设置中打开 Web 访问演示，再点击“打开远程端预览”。也可直接打开 `index.html?view=remote`。点击登录页展示的演示令牌即可自动填入。

终端输入、会话生命周期和远程连接均为本地模拟，不执行命令、不调用模型、不修改项目文件。项目、标签和 Agent 开关保存在浏览器的 `vibe-design-v1` 本地存储中；源码编辑仅保存在本次页面草稿中。

## 预览图

- [终端主界面](screenshots/desktop.png)
- [源码预览](screenshots/code-preview.png)
- [图片路径预览](screenshots/image-preview.png)
- [源代码控制](screenshots/source-control.png)
- [Agent 设置与图标](screenshots/settings-agents.png)
- [对话记录](screenshots/transcript.png)
- [远程访问设置](screenshots/settings-remote.png)
- [手机会话列表](screenshots/mobile-sessions.png)
- [手机终端](screenshots/mobile-terminal.png)

已在 Edge 无头浏览器验证工作区添加与表单校验、标签重命名、终端输入、停止与恢复、Agent 显隐、源码未保存保护、远程登录与输入、远程菜单、本地持久化，以及桌面和手机多种视口。所有数据均来自隔离的原型演示。
