'use strict';
// Isolated design data only. No backend, process execution, or project-file writes.
const $ = (s, root = document) => root.querySelector(s);
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const paths = {
  panel:'M3 4h18v16H3ZM15 4v16', branch:'M6 3v12a4 4 0 0 0 8 0V9M4 3a2 2 0 1 0 4 0 2 2 0 0 0-4 0M12 7a2 2 0 1 0 4 0 2 2 0 0 0-4 0M4 19a2 2 0 1 0 4 0 2 2 0 0 0-4 0',
  terminal:'m4 6 6 6-6 6m9 0h7', plus:'M12 5v14M5 12h14', close:'m6 6 12 12M6 18 18 6',
  folder:'M3 7h7l2 2h9v11H3ZM3 7V4h7l2 3', grid:'M3 3h7v7H3ZM14 3h7v7h-7ZM3 14h7v7H3ZM14 14h7v7h-7Z',
  code:'m8 5-6 7 6 7m8-14 6 7-6 7m-3-16-2 18', more:'M5 12h.01M12 12h.01M19 12h.01',
  settings:'M9 3h6l1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1ZM9 12a3 3 0 1 0 6 0 3 3 0 0 0-6 0',
  remote:'M5 2h14v20H5ZM10 18h4', book:'M4 3h16v18H4ZM8 7h8M8 11h8M8 15h5',
  arrow:'M5 12h14m-6-6 6 6-6 6', up:'M12 19V5m-6 6 6-6 6 6', down:'M12 5v14m-6-6 6 6 6-6',
  back:'M19 12H5m6-6-6 6 6 6', copy:'M8 8h13v13H8ZM16 8V3H3v13h5', check:'m5 12 4 4L19 6',
  refresh:'M20 8a9 9 0 1 0 1 8M20 3v6h-6', image:'M3 3h18v18H3Zm0 14 6-6 5 5 3-3 4 4M15 7h.01',
  play:'m8 4 12 8-12 8Z', stop:'M6 6h12v12H6Z', chevron:'m9 5 7 7-7 7', keyboard:'M2 5h20v14H2ZM6 9h.01M10 9h.01M14 9h.01M18 9h.01M6 13h.01M10 13h.01M14 13h.01M18 13h.01M7 16h10',
  shield:'M12 2 3 6v6c0 5 9 10 9 10s9-5 9-10V6Zm-5 10 3 3 7-7', wrap:'M3 6h18M3 11h14a4 4 0 0 1 0 8h-5m3-3-3 3 3 3M3 16h4', edit:'m4 16-1 5 5-1L21 7l-5-5ZM13 5l5 5', bolt:'m13 2-9 12h7l-1 8 10-12h-7Z'
};
const icon = name => `<svg class="ico" viewBox="0 0 24 24" aria-hidden="true"><path d="${paths[name] || paths.terminal}"/></svg>`;
const btn = (action, text, ico, cls = '', attrs = '') => `<button type="button" class="${cls}" data-action="${action}" ${attrs}>${ico ? icon(ico) : ''}${text ? `<span>${text}</span>` : ''}</button>`;
const agentClass = type => ({'Claude Code':'claude','Codex':'codex','PowerShell':'power','Gemini CLI':'gemini'}[type] || '');
const agentImages = {'Pi':'pi.svg','Codex':'codex.svg','Claude Code':'claude.svg','Gemini CLI':'gemini.png'};
const agent = type => `<span class="agent ${agentClass(type)}">${agentImages[type] ? `<img src="icons/${agentImages[type]}" alt="" aria-hidden="true" width="16" height="16">` : icon('terminal')}</span>`;
const status = tab => `<span class="badge"><i class="dot ${tab.running ? 'live' : ''}"></i>${tab.running ? '运行中' : '休眠'}</span>`;
const seed = () => ({
  selected:'vibe', web:false, hidden:[],
  workspaces:[
    {id:'vibe',name:'vibe-terminal',path:'E:/WorkSpace/vibe-termial',selected:'pi',tabs:[{id:'pi',name:'界面优化',type:'Pi',running:true,logs:[]},{id:'codex',name:'会话恢复',type:'Codex',running:true,logs:[]},{id:'shell',name:'PowerShell',type:'PowerShell',running:false,logs:[]}]},
    {id:'studio',name:'vanceai-studio',path:'E:/WorkSpace/vanceai-studio',selected:'claude',tabs:[{id:'claude',name:'Claude Code',type:'Claude Code',running:false,logs:[]}]},
    {id:'notes',name:'playground',path:'E:/WorkSpace/playground',selected:null,tabs:[]}
  ]
});
let state;
try { state = JSON.parse(localStorage.getItem('vibe-design-v1')); } catch {}
if (!state?.workspaces || !Array.isArray(state.workspaces) || !state.workspaces.length) state = seed();
state.collapsedWorkspaces ||= [];
let page = 'terminal', preview = null, editing = false, dirty = false, wrap = false, remoteConnected = false, remoteTab = null, remoteWorkspace = null;
let imageURL = '', imageName = 'workspace-cover.png', modalOp = null, pending = null, lastFocus = null, toastTimer, refreshTimer;
const standaloneRemote = new URLSearchParams(location.search).get('view') === 'remote';
const ws = () => state.workspaces.find(w => w.id === state.selected);
const tab = () => ws()?.tabs.find(t => t.id === ws().selected);
const save = () => { try { localStorage.setItem('vibe-design-v1', JSON.stringify(state)); } catch { toast('浏览器存储不可用，本次交互会保留到关闭页面。'); } };
let code = `use gpui_kit::*;
use crate::session::SessionManager;

pub struct WorkspaceView {
    session: SessionManager,
    preview: Option<PreviewContent>,
}

impl WorkspaceView {
    pub fn render_workspace(&self) {
        // Keep the terminal in focus.
        let workspace = self.session.workspace();

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.tab_bar())
            .child(
                div()
                    .flex()
                    .flex_1()
                    .child(self.terminal())
                    .when_some(self.preview(), |ui, pane| {
                        ui.child(pane)
                    }),
            )
    }
}`;
let savedCode = code;
let rightOpen = false, rightTab = 'explorer', panelWidth = null, activeFile = 'src/ui.rs';
const panelTabs = [{id:'explorer',label:'资源管理器',render:explorerHTML}, {id:'scm',label:'源代码控制',render:scmHTML}];
const explorerFolders = new Set(['src','assets']);
const sampleFiles = {
  'src/ui.rs':{kind:'code',language:'Rust',content:code},
  'src/session.rs':{kind:'code',language:'Rust',content:'pub struct SessionManager {\n    pub selected_workspace: Option<WorkspaceId>,\n}\n\nimpl SessionManager {\n    pub fn resume(&mut self, tab_id: TabId) {\n        // Restore the session bound to this tab.\n        self.launch(tab_id);\n    }\n}'},
  'src/terminal.rs':{kind:'code',language:'Rust',content:'pub struct Terminal {\n    rows: u16,\n    columns: u16,\n    running: bool,\n}\n\nimpl Terminal {\n    pub fn resize(&mut self, rows: u16, columns: u16) {\n        self.rows = rows;\n        self.columns = columns;\n    }\n}'},
  'README.md':{kind:'readme',language:'Markdown'},
  'assets/workspace-cover.png':{kind:'image',language:'图片'},
  'Cargo.toml':{kind:'code',language:'TOML',content:'[package]\nname = "vibe-terminal"\nversion = "0.1.0"\nedition = "2024"\n\n# Project manifest preview fixture.\n[dependencies]\nserde = { version = "1", features = ["derive"] }'}
};
const demoChanges = [{path:'src/ui.rs',status:'M',staged:false},{path:'README.md',status:'M',staged:false},{path:'docs/prototype/index.html',status:'U',staged:false}];
let selectedChange = 'src/ui.rs';
const availableAgents = ['Pi','Codex','Claude Code','Gemini CLI'];
function toast(message) { clearTimeout(toastTimer); $('#toast').textContent = message; $('#toast').hidden = false; toastTimer = setTimeout(() => $('#toast').hidden = true, 3200); }
function closeMenu() { $('#menu').hidden = true; }
function guarded(fn) {
  if (!dirty) return fn();
  pending = fn;
  openModal('保存文件修改？', `<p>${esc(activeFile)} 有尚未保存的修改。保存后再继续，或放弃这次修改。</p>`, `${btn('cancel-modal','继续编辑',null)}${btn('discard','不保存',null,'danger')}${btn('save-continue','保存并继续',null,'primary')}`, 'unsaved');
}
function render() {
  if (standaloneRemote) { $('#root').innerHTML = '<main class="standalone-remote">' + remoteHTML() + '</main>'; return; }
  const w = ws(), t = tab();
  const projects = state.workspaces.map(item => {
    const active = item.id === state.selected;
    const expanded = !state.collapsedWorkspaces.includes(item.id);
    return '<section class="project-group"><div class="workspace ' + (active ? 'active' : '') + '">' +
      btn('toggle-workspace','','chevron','workspace-toggle','data-id="' + esc(item.id) + '" aria-label="' + (expanded ? '折叠 ' : '展开 ') + esc(item.name) + '" aria-expanded="' + expanded + '" aria-controls="workspace-sessions-' + esc(item.id) + '"') +
      '<button type="button" class="workspace-main" data-action="workspace" data-id="' + esc(item.id) + '" title="' + esc(item.path) + '">' + icon('folder') + '<span class="workspace-text"><strong>' + esc(item.name) + '</strong></span></button>' +
      btn('workspace-menu','','more','icon workspace-more','data-id="' + esc(item.id) + '" aria-label="' + esc(item.name) + ' 操作"') + '</div>' +
      ('<div class="project-sessions" id="workspace-sessions-' + esc(item.id) + '"' + (expanded ? '' : ' hidden') + '>' + item.tabs.map(session => '<button class="sidebar-session ' + (active && item.selected === session.id && page !== 'overview' ? 'active' : '') + '" data-action="tab" data-workspace="' + esc(item.id) + '" data-id="' + esc(session.id) + '" title="' + esc(session.type) + '">' + agent(session.type) + '<span>' + esc(session.name) + '</span><i class="dot ' + (session.running ? 'live' : '') + '"></i></button>').join('') + '</div>') + '</section>';
  }).join('');
  const tabs = w ? w.tabs.map(item => '<div class="tab ' + (page !== 'overview' && item.id === w.selected ? 'active' : '') + '" data-tab="' + esc(item.id) + '"><button type="button" data-action="tab" data-id="' + esc(item.id) + '" title="双击重命名" ' + (page !== 'overview' && item.id === w.selected ? 'aria-current="page"' : '') + '>' + agent(item.type) + esc(item.name) + '</button>' + btn('close-tab','','close','close','data-id="' + esc(item.id) + '" aria-label="关闭 ' + esc(item.name) + '"') + '</div>').join('') : '';
  $('#root').innerHTML = '<div class="shell terminal-first">' +
    '<aside class="sidebar" aria-label="项目工作区"><div class="sidebar-app"><div class="brand">' + icon('terminal') + '<span>VibeTerm</span></div></div>' +
    '<div class="sidebar-label"><span>项目</span>' + btn('add-workspace','','plus','icon','aria-label="添加工作区" title="添加工作区"') + '</div><nav class="workspace-list">' + projects + '</nav>' +
    '<div class="sidebar-bottom">' + btn('settings','设置','settings','','aria-label="设置" title="设置"') + '</div></aside>' +
    '<main class="content" id="main" tabindex="-1"><div class="tabsbar"><nav class="tabs" aria-label="终端标签">' + tabs + (w ? btn('new-menu','','plus','icon','aria-label="新建终端" title="选择终端"') : '') + '</nav>' + (t ? '<div class="tab-tools">' + btn(page === 'transcript' ? 'terminal' : 'transcript','','book',page === 'transcript' ? 'icon selected' : 'icon','aria-label="' + (page === 'transcript' ? '返回终端' : '查看内容') + '" title="' + (page === 'transcript' ? '返回终端' : '查看内容') + '"') + btn('session-menu','','more','icon','aria-label="会话操作" title="会话操作"') + '</div>' : '') + btn('toggle-panel','','panel',rightOpen ? 'icon panel-toggle selected' : 'icon panel-toggle','aria-label="' + (rightOpen ? '折叠右侧面板' : '打开右侧面板') + '" title="' + (rightOpen ? '折叠右侧面板' : '打开右侧面板') + '" aria-expanded="' + rightOpen + '"') + '</div>' +
    '<div class="workarea">' + (w ? page === 'overview' ? overviewHTML() : page === 'transcript' ? transcriptHTML() : terminalHTML() : '<div class="empty">' + icon('terminal') + '<h2>添加一个项目</h2><p>在项目目录中启动终端。</p>' + btn('add-workspace','添加项目','plus','outline') + '</div>') + (rightOpen ? rightPanelHTML() : '') + '</div></main>' +
    '<footer class="statusbar"><span class="flex"><i class="dot ' + (t?.running ? 'live' : '') + '"></i>' + (t ? esc(t.type) : 'VibeTerm') + '</span><span class="mono grow">' + esc(w?.path || '') + '</span><span class="secondary-status">UTF8</span><span>原型演示</span></footer></div>';
  const out = $('#terminal-output'); if (out && t?.logs.length) out.scrollTop = out.scrollHeight;
}
function overviewHTML() {
  const w = ws();
  return '<section class="overview"><div class="overview-project">' + icon('folder') + '<h2>' + esc(w.name) + '</h2></div><p class="overview-directory mono muted">' + esc(w.path) + '</p><div class="section-heading"><span class="muted">终端 · ' + w.tabs.length + '</span>' + btn('new-menu','新建','plus') + '</div>' +
    (w.tabs.length ? '<div class="session-list">' + w.tabs.map(t => '<article class="session-row">' + agent(t.type) + '<div class="grow"><strong>' + esc(t.name) + '</strong><small>' + esc(t.type) + '</small></div>' + status(t) + btn('tab',t.running ? '打开' : '恢复','arrow','','data-id="' + esc(t.id) + '"') + '</article>').join('') + '</div>' : '<div class="empty">' + icon('terminal') + '<p>这个项目还没有终端</p>' + btn('new-menu','新建终端','plus','outline') + '</div>') +
    '<div class="overview-foot">' + btn('copy-directory','复制项目路径','copy') + '</div></section>';
}
function terminalHTML() {
  const t = tab();
  if (!t) return '<div class="terminal-pane"><div class="empty">' + icon('terminal') + '<p>选择一个 Shell 或 Agent 开始</p>' + btn('new-menu','新建终端','plus','outline') + '</div></div>';
  const shell = ['PowerShell','CMD'].includes(t.type);
  let output = '';
  if (t.id === 'pi') output = '<div class="term-block"><div class="term-prompt"><span>❯</span>基于现有功能，生成一份可以交互的 HTML 原型。</div></div>' +
    '<div class="term-block"><p class="term-paragraph">先检查工作区、会话和文件预览的现有实现。</p><div class="term-task"><span class="check">●</span><strong>Read</strong> <button class="file-link" data-action="preview-code">src/ui.rs:12</button></div><div class="tool-detail">工作区布局、终端标签与预览面板</div>' +
    '<div class="term-task"><span class="check">●</span><strong>Read</strong> <button class="file-link" data-action="preview-readme">README.md</button></div><div class="tool-detail">项目功能与会话恢复说明</div></div>' +
    '<div class="term-block"><div class="term-task"><span class="check">●</span><strong>Bash</strong> rg --files src</div><pre class="code-output">  src/ui.rs          src/session.rs\n  src/terminal.rs    src/transcript.rs\n  src/paths.rs       src/agents.rs</pre></div>' +
    '<div class="term-block"><p class="term-paragraph">保留现有工作区、终端和预览逻辑，界面收敛为：</p><pre class="code-output">  项目导航     按项目切换终端与 Agent\n  终端标签     各自保存独立的会话上下文\n  文件预览     点击输出中的路径后展开</pre></div>' +
    '<div class="term-block"><div class="term-task"><span class="check">●</span><strong>Read</strong> <button class="file-link" data-action="preview-image">assets/workspace-cover.png</button></div><div class="tool-detail">图片与源码通过同一个侧面板预览</div></div>' +
    '<p class="term-paragraph">已梳理完成。可以点击上面的文件路径查看内容。</p><p class="terminal-completion muted">Worked for 24s</p>';
  else if (shell) output = '<pre class="code-output shell-output">' + (t.type === 'PowerShell' ? 'Windows PowerShell\nCopyright (C) Microsoft Corporation. All rights reserved.\n\nPS ' : '') + esc(ws().path) + '&gt; pwd\n\nPath\n' + esc(ws().path) + '</pre>';
  else output = '<div class="term-prompt"><span>❯</span>' + (t.type === 'Codex' ? '检查每个终端标签的会话恢复逻辑。' : '继续这个项目。') + '</div><div class="term-task"><span class="check">●</span><strong>Read</strong> <button class="file-link" data-action="preview-code">src/ui.rs:12</button></div><p class="term-paragraph">每个标签使用独立会话 ID，恢复时继续对应的上下文。</p>';
  const logs = (t.logs || []).map(log => '<div class="term-block"><div class="term-prompt"><span>❯</span><span style="color:inherit;white-space:pre-wrap">' + esc(log.input) + '</span></div><p class="term-paragraph" style="white-space:pre-wrap">' + esc(log.output) + '</p></div>').join('');
  return '<section class="terminal-pane" aria-label="终端会话"><div class="terminal-output" id="terminal-output" tabindex="0" aria-label="终端输出，可选择复制">' +
    '<div class="cli-heading"><strong>' + agent(t.type) + esc(t.type) + '</strong><span class="muted">' + esc(ws().path) + '</span></div>' + output + logs +
    (!t.running ? '<div class="terminal-note">' + icon('stop') + '会话已停止，历史已保留。' + btn('resume','恢复','play','outline') + '</div>' : '') +
    '<form class="composer" id="terminal-form"><span class="prompt">' + (shell ? '>' : '❯') + '</span><textarea id="command" rows="1" aria-label="终端输入" placeholder="' + (t.running ? '输入命令或消息…' : '恢复会话后继续输入') + '" ' + (!t.running ? 'disabled' : '') + '></textarea><button type="submit" class="inline-submit" aria-label="发送" ' + (!t.running ? 'disabled' : '') + '>' + icon('arrow') + '</button></form><div class="cli-input-hint">' + (shell ? 'Enter 执行' : 'Enter 发送 · Shift + Enter 换行') + '</div></div></section>';
}
function highlight(line) {
  // Illustrative, deliberately small tokenizer for the sample snippet.
  return line.split(/(\/\/.*$|"[^"\n]*"|\b(?:use|pub|struct|impl|fn|let|self|Some|Option)\b)/g).map(part => {
    const cls = part.startsWith('//')?'comment':part.startsWith('"')?'str':/^(use|pub|struct|impl|fn|let|self|Some|Option)$/.test(part)?'kw':'';
    return cls?`<span class="${cls}">${esc(part)}</span>`:esc(part);
  }).join('');
}
function rightPanelHTML() {
  const selectedPanel = panelTabs.find(item => item.id === rightTab) || panelTabs[0];
  const contents = selectedPanel.render();
  return '<aside class="right-panel" aria-label="右侧面板"' + (panelWidth ? ' style="width:' + panelWidth + 'px"' : '') + '><div class="resize-handle" role="separator" aria-label="调整右侧面板宽度" aria-orientation="vertical" tabindex="0"></div><nav class="right-tabs" role="tablist" aria-label="面板视图">' + panelTabs.map(item => '<button type="button" role="tab" id="panel-tab-' + item.id + '" aria-controls="panel-content" aria-selected="' + (rightTab === item.id) + '" data-action="panel-tab" data-id="' + item.id + '" class="' + (rightTab === item.id ? 'active' : '') + '">' + item.label + '</button>').join('') + '</nav><div class="panel-content" id="panel-content" role="tabpanel" aria-labelledby="panel-tab-' + rightTab + '">' + contents + '</div></aside>';
}
function explorerHTML() {
  const selectedPath = preview === 'image' ? 'assets/workspace-cover.png' : preview === 'readme' ? 'README.md' : preview === 'code' ? activeFile : null;
  const fileRow = file => '<button class="tree-file ' + (selectedPath === file ? 'active' : '') + '" data-action="explorer-file" data-file="' + esc(file) + '">' + icon(file.endsWith('.png') ? 'image' : 'code') + '<span>' + esc(file.split('/').pop()) + '</span></button>';
  const folder = (name, files) => '<div class="tree-folder"><button class="tree-folder-name" data-action="explorer-folder" data-folder="' + name + '" aria-expanded="' + explorerFolders.has(name) + '"><span class="tree-arrow">' + (explorerFolders.has(name) ? '⌄' : '›') + '</span>' + icon('folder') + '<span>' + name + '</span></button>' + (explorerFolders.has(name) ? '<div class="tree-children">' + files.map(fileRow).join('') + '</div>' : '') + '</div>';
  return '<div class="file-tree" aria-label="项目文件"><div class="tree-project">' + esc(ws()?.name) + '</div>' + folder('src',['src/ui.rs','src/session.rs','src/terminal.rs']) + folder('assets',['assets/workspace-cover.png']) + fileRow('Cargo.toml') + fileRow('README.md') + '</div><div class="file-detail">' + (preview ? previewHTML() : '<div class="panel-empty">' + icon('code') + '<p>选择文件查看内容</p><span>也可以点击终端中的文件路径</span></div>') + '</div>';
}
function scmHTML() {
  const group = (staged, label) => {
    const changes = demoChanges.filter(change => change.staged === staged);
    return '<section class="change-group"><div class="change-heading"><span>' + label + '</span><span>' + changes.length + '</span></div>' + (changes.length ? changes.map(change => '<div class="change-row ' + (selectedChange === change.path ? 'active' : '') + '"><button class="change-file" data-action="scm-file" data-file="' + esc(change.path) + '">' + icon('code') + '<span>' + esc(change.path) + '</span><span class="change-status ' + (change.status === 'U' ? 'untracked' : '') + '">' + change.status + '</span></button>' + btn('scm-stage','',staged?'close':'plus','icon','data-file="' + esc(change.path) + '" aria-label="' + (staged?'取消暂存 ':'暂存 ') + esc(change.path) + '" title="' + (staged?'取消暂存':'暂存更改') + '"') + '</div>').join('') : '<p class="change-empty">暂无' + label + '</p>') + '</section>';
  };
  const lines = selectedChange === 'src/ui.rs' ? [
    ['context','fn render_workspace(&self) {'],['remove','    self.render_workspace_header();'],['add','    self.render_terminal_tabs();'],['context','    self.render_terminal();'],['remove','    self.render_image_button();'],['add','    self.render_right_panel();'],['context','}']
  ] : selectedChange === 'README.md' ? [['context','# VibeTerm'],['remove','项目与终端工作区'],['add','项目、终端与按需展开的文件面板'],['context',''],['add','右侧支持资源管理器与源代码控制视图。']] : [['add','<!doctype html>'],['add','<html lang="zh-CN">'],['add','  <title>VibeTerm</title>'],['add','  <main id="workspace"></main>'],['add','</html>']];
  return '<div class="scm-changes"><div class="branch-label">' + icon('branch') + '<span>main</span><span class="muted small">演示工作树</span></div>' + group(true,'暂存的更改') + group(false,'更改') + '</div><div class="diff-detail"><div class="diff-heading">' + icon('code') + '<strong>' + esc(selectedChange) + '</strong><span>差异预览</span></div><div class="diff-code">' + lines.map(([kind,line],index) => '<div class="diff-line ' + kind + '"><span class="diff-number">' + (index+1) + '</span><span class="diff-sign">' + (kind==='add'?'+':kind==='remove'?'−':' ') + '</span><code>' + esc(line) + '</code></div>').join('') + '</div><p class="diff-note">暂存与差异均为原型演示</p></div>';
}
function previewHTML() {
  const isImage = preview === 'image', isReadme = preview === 'readme', filename = isImage ? imageName : isReadme?'README.md':activeFile.split('/').pop();
  const content = isReadme ? '# VibeTerm\n\nA focused workspace for your coding agents.\n\n## Features\n\n• Multiple project workspaces\n• Independent terminal sessions\n• Local source and image previews\n• Pi and Codex conversation history\n• LAN and Tailscale remote access\n\n## Run\n\ncargo run --locked' : code;
  return `<aside class="preview" aria-label="文件预览"><div class="preview-heading">${icon(isImage?'image':'code')}<strong>${esc(filename)}${dirty?' <span class="mint">●</span>':''}</strong>${btn('close-preview','','close','icon','aria-label="关闭预览"')}</div><div class="preview-path" title="${esc(ws()?.path)}">${esc(ws()?.path)}/${isImage?'assets/'+esc(imageName):isReadme?'README.md':esc(activeFile)}</div>
  <div class="preview-toolbar"><span class="badge">${isImage?'图片':isReadme?'Markdown':esc(sampleFiles[activeFile]?.language||'Text')}</span>${!isImage?btn('wrap','', 'wrap',wrap?'selected':'',`aria-label="自动换行" title="自动换行" aria-pressed="${wrap}"`):''}${!isImage&&!isReadme?btn('edit',editing?'完成':'编辑','edit',editing?'selected':''):''}${btn('copy-preview',isImage?'路径':'复制','copy')}</div>
  ${isImage?`<div class="preview-image">${imageURL?`<img src="${imageURL}" alt="${esc(imageName)}">`:`<div class="image-art" role="img" aria-label="VibeTerm 品牌示例图片"><span class="art-mark">&gt;_</span><div><h2>Less noise.<br>More flow.</h2><p style="margin-top:16px">VibeTerm / Your local workspace.</p></div><span class="small mono">DESIGN EXPLORATION · 01</span></div>`}</div>`:editing&&!isReadme?`<textarea class="editor" id="code-editor" aria-label="源码编辑器" spellcheck="false" wrap="${wrap?'soft':'off'}">${esc(code)}</textarea>`:`<div class="code-scroll ${wrap?'wrap':''}">${content.split('\n').map((line,i)=>`<div class="code-line ${!isReadme&&i===11?'target':''}"><span class="line-number">${i+1}</span><span class="code-source">${highlight(line)}</span></div>`).join('')}</div>`}
  <div class="preview-footer"><span>${isImage?'适应面板':editing?dirty?'未保存 · Ctrl + S 保存':'已保存至原型草稿': '只读预览'}</span><span class="mono">${isImage?'本地图片':isReadme?'UTF8':'Ln 12, Col 9'}</span></div></aside>`;
}
function transcriptHTML() {
  const t = tab();
  if (!t || !['Pi','Codex'].includes(t.type)) return `<section class="terminal-pane"><div class="empty">${icon('book')}<h2>这个会话暂不支持内容视图</h2><p>目前可读取 Pi 与 Codex 落盘的对话记录。<br>其他 CLI 可以继续使用终端视图。</p>${btn('terminal','返回终端','back','outline')}</div></section>`;
  return `<section class="terminal-pane"><div class="pane-meta">${icon('book')}对话记录 <span class="badge">${3+(t.logs?.length||0)*2} 条消息</span>${btn('refresh-transcript','刷新','refresh')}</div><div class="transcript"><h2>${esc(t.name)}</h2><div class="source">${t.type==='Pi'?`sessions/${esc(t.id)}.jsonl`:`~/.codex/sessions/2026/11/rollout-${esc(t.id)}.jsonl`}<br>本机会话文件 · 原型示例</div><article class="message user"><div class="message-head"><strong>你</strong><small>14:32</small></div><p>${t.type==='Pi'?'梳理工作区布局，让终端和代码预览更清晰。':'检查每个终端标签的会话恢复逻辑。'}</p></article><article class="message"><div class="message-head">${agent(t.type)}<strong>${esc(t.type)}</strong><small>14:33</small></div><p>我会先阅读现有的工作区和会话实现，保留每个标签独立的上下文，然后调整界面的信息层级。</p><div class="tool-result"><span class="mint">✓ read</span> src/ui.rs<br>已读取工作区布局、标签切换与文件预览。</div></article><article class="message"><div class="message-head">${agent(t.type)}<strong>${esc(t.type)}</strong><small>14:34</small></div><p>终端作为主工作区，文件以侧面板呈现。需要追溯讨论时，再切换到内容视图。</p><button class="file-link" data-action="preview-code">查看 src/ui.rs:12 ${icon('arrow')}</button></article>${(t.logs||[]).map(log=>`<article class="message user"><div class="message-head"><strong>你</strong><small>刚刚</small></div><p>${esc(log.input)}</p></article><article class="message"><div class="message-head">${agent(t.type)}<strong>${esc(t.type)}</strong><small>刚刚</small></div><p>${esc(log.output)}</p></article>`).join('')}</div></section>`;
}
function showMenu(button, items) {
  const menu = $('#menu');
  ($('#modal').open ? $('#modal') : document.body).appendChild(menu);
  menu.innerHTML = items; menu.hidden = false;
  const rect = button.getBoundingClientRect();
  menu.style.left = `${Math.max(8,Math.min(rect.left,innerWidth-menu.offsetWidth-8))}px`;
  menu.style.top = `${Math.max(8,Math.min(rect.bottom+8,innerHeight-menu.offsetHeight-8))}px`;
  $('button',menu)?.focus();
}
function newMenu(button) {
  showMenu(button,`<div class="menu-label">新建终端</div>${['CMD','PowerShell'].map(type=>`<button data-action="create-tab" data-type="${type}">${agent(type)}${type}<small>Shell</small></button>`).join('')}<div class="separator"></div><div class="menu-label">已检测到的 Agent · 演示</div>${availableAgents.filter(type=>!state.hidden.includes(type)).map(type=>`<button data-action="create-tab" data-type="${type}">${agent(type)}${type}<small>Agent</small></button>`).join('')}<div class="separator"></div>${btn('settings','管理 Agent','settings')}`);
}
function openModal(title,body,footer,op='',cls='') {
  closeMenu();
  document.body.appendChild($('#menu'));
  if (!$('#modal').open) lastFocus = document.activeElement;
  $('#modal').className = cls;
  $('#modal').innerHTML = `<div class="modal-heading"><h2 id="modal-title">${title}</h2>${btn('cancel-modal','','close','icon','aria-label="关闭对话框"')}</div><div class="modal-body">${body}</div>${footer?`<div class="modal-footer">${footer}</div>`:''}`;
  modalOp = op; if (!$('#modal').open) $('#modal').showModal();
  requestAnimationFrame(()=>($('input:not([type=hidden])',$('#modal'))||$('button',$('#modal')))?.focus());
}
function closeModal() { closeMenu(); document.body.appendChild($('#menu')); $('#modal').close(); modalOp = null; pending = null; if(!standaloneRemote)render(); lastFocus?.isConnected && lastFocus.focus(); }
function settings(section='agents') {
  const nav = [['agents','Agent','terminal'],['remote','远程访问','remote'],['shortcuts','快捷键','keyboard']];
  openModal('设置', '', '', 'settings','settings-modal');
  $('.modal-body',$('#modal')).outerHTML = `<div class="settings-layout"><nav class="settings-nav" aria-label="设置分类">${nav.map(([id,label,ico])=>btn('settings-tab',label,ico,id===section?'active':'',`data-id="${id}" ${id===section?'aria-current="page"':''}`)).join('')}</nav><section class="settings-pane" id="settings-pane">${settingsPane(section)}</section></div>`;
}
function settingsPane(section) {
  if(section==='agents') return `<div class="between"><h3>你的 coding agents</h3>${btn('scan-agents','重新检测','refresh','outline')}</div><p>自动发现本机安装的 Agent。启用后，它会出现在新建终端菜单中。</p><div id="agent-rows">${availableAgents.map(type=>`<div class="agent-row">${agent(type)}<div class="grow"><strong>${type}</strong><code>${type==='Pi'?'~/.pi/agent/bin/pi':type==='Claude Code'?'~/.local/bin/claude':`~/AppData/Roaming/npm/${type==='Codex'?'codex':'gemini'}`}</code></div><button class="switch" role="switch" aria-checked="${!state.hidden.includes(type)}" aria-label="启用 ${type}" data-action="toggle-agent" data-type="${type}"></button></div>`).join('')}</div><p>这里展示检测结果示例。CMD 与 PowerShell 固定在新建终端菜单的顶部。</p>`;
  if(section==='remote') return `<h3>从另一块屏幕，继续。</h3><p>在同一局域网或 Tailscale 网络中，通过浏览器访问本机的终端。</p><div class="setting-card"><div class="between"><strong>Web 访问</strong><button class="switch" role="switch" aria-checked="${state.web}" aria-label="开启 Web 访问" data-action="toggle-web"></button></div><p>${state.web?'已开启演示状态，可体验远程端的连接流程。':'开启后显示访问地址和令牌。电脑需保持运行。'}</p></div>${state.web?`<div class="setting-card"><strong>局域网地址</strong><div class="remote-address"><code>http://192.168.1.86:8787</code>${btn('copy-lan','','copy','icon','aria-label="复制局域网链接"')}</div><p>演示地址 · 实际地址由桌面应用生成</p></div><div class="setting-card"><div class="between"><strong>Tailscale 地址</strong>${btn('retry-tailscale','重新获取','refresh')}</div><div class="remote-address"><code>http://100.101.32.16:8787</code>${btn('copy-tailnet','','copy','icon','aria-label="复制 Tailscale 链接"')}</div></div><div class="setting-card"><strong>访问令牌</strong><div class="remote-address"><code>vibe-demo-token</code>${btn('copy-token','','copy','icon','aria-label="复制访问令牌"')}</div><p>此令牌仅用于原型演示。</p></div>${btn('remote-demo','打开远程端预览','remote','primary')}`:''}<p>桌面与浏览器共享同一会话。断开浏览器连接，会话继续运行。</p>`;
  return `<h3>让操作跟上思路。</h3><p>保持现有快捷键，常用操作随手可达。</p>${[['新建默认终端','Ctrl + T'],['关闭当前终端','Ctrl + W'],['下一个 / 上一个标签','Ctrl + Tab'],['切换到第 N 个标签','Ctrl + 1 … 9'],['设置','Ctrl + ,'],['快捷键','F1'],['复制选中输出','Ctrl + Shift + C'],['打开文件预览','Ctrl + 点击'],['保存文件','Ctrl + S'],['关闭面板或对话框','Esc']].map(([label,key])=>`<div class="shortcut-row"><span>${label}</span><kbd class="key">${key}</kbd></div>`).join('')}<p>浏览器会优先处理一部分系统快捷键。原型也支持双击标签重命名。</p>`;
}
function workspaceDialog(item) {
  openModal(item?'重命名工作区':'添加工作区',`<p>${item?'使用易于识别的名称来区分项目。':'选择一个项目目录，终端将在该目录启动。原型中使用示例路径。'}</p><form id="workspace-form" class="stack"><label>项目名称<input id="workspace-name" value="${esc(item?.name||'')}" placeholder="例如 my-project" maxlength="64" required></label>${item?'':`<label>项目目录<input id="workspace-path" value="E:/WorkSpace/" placeholder="E:/WorkSpace/my-project" required></label>`}<p class="field-error" id="form-error" role="alert"></p></form>`,`${btn('cancel-modal','取消',null)}<button class="primary" type="submit" form="workspace-form">${item?'保存名称':'添加项目'}</button>`,item?`workspace:${item.id}`:'add-workspace');
}
function sendInput(input, target=tab()) {
  if (!input.trim() || !target?.running) return;
  const out = /^(pwd|Get-Location)$/i.test(input.trim())?ws()?.path:input.trim()==='help'?'试试 pwd、clear，或点击文件路径、切换内容视图。其他输入会展示演示回复。':'已收到这条输入。这是交互原型的演示回复，没有执行命令或调用模型。';
  if(input.trim()==='clear') target.logs=[];
  else { target.logs ||= []; target.logs.push({input,output:out}); }
  save(); render(); if($('#modal').open&&modalOp==='remote') renderRemote(); $('#command')?.focus();
}
async function copy(text) {
  try { await navigator.clipboard.writeText(text); toast('已复制到剪贴板'); }
  catch { openModal('复制内容',`<p>浏览器未允许访问剪贴板，可以选中下方内容手动复制。</p><textarea id="copy-value" rows="3" readonly aria-label="待复制内容">${esc(text)}</textarea>`,btn('cancel-modal','关闭',null,'primary'),'copy'); $('#copy-value').select(); }
}
function openPreview(type, file) { guarded(()=>{ if(type==='code'){activeFile=file||'src/ui.rs';code=sampleFiles[activeFile]?.content||code;savedCode=code;}preview=type;rightOpen=true;rightTab='explorer';editing=false;dirty=false;render(); }); }
function saveDraft() { code=$('#code-editor')?.value??code; savedCode=code; if(sampleFiles[activeFile])sampleFiles[activeFile].content=code; dirty=false; toast('已保存到本次原型草稿'); }
function renderRemote() {
  if(standaloneRemote) return render();
  const root=$('#remote-root'); if(root) root.innerHTML=remoteHTML();
  const out=$('.remote-output'); if(out) out.scrollTop=out.scrollHeight;
}
function getRemoteTab() { return state.workspaces.find(w=>w.id===remoteWorkspace)?.tabs.find(t=>t.id===remoteTab); }
function remoteHTML() {
  const t=getRemoteTab();
  return `<div class="remote-frame"><header class="remote-top">${remoteConnected&&t?btn('remote-list','','back','icon','aria-label="返回会话列表"'):''}<div class="brand"><img src="mark.svg" alt="">${t?esc(t.name):'VibeTerm'}</div>${remoteConnected?`<span class="badge"><i class="dot live"></i>演示连接</span>`:''}</header>
  ${!remoteConnected?`<div class="remote-scroll"><div class="remote-login"><img src="mark.svg" alt=""><div><h1>换个屏幕。<br>接着写。</h1><p style="margin-top:16px">连接电脑上的终端，<br>让正在进行的工作继续。</p></div><form id="remote-login"><label>访问令牌<input id="remote-token" placeholder="粘贴桌面端的访问令牌" autocomplete="off"></label><span id="remote-error" class="field-error" role="alert"></span><button type="submit" class="primary">连接工作台 ${icon('arrow')}</button></form><p class="small">原型演示令牌：<button class="file-link small" data-action="fill-token">vibe-demo-token</button><br>此页面仅模拟连接流程。</p></div></div>`:!t?`<div class="remote-scroll"><div class="between" style="margin-bottom:24px"><h2>你的会话</h2><span class="small muted">Windows 本机</span></div>${state.workspaces.map(w=>`<section class="remote-workspace"><div class="eyebrow">${esc(w.name)}</div>${w.tabs.map(t=>`<button class="remote-session" data-action="remote-select" data-id="${esc(t.id)}" data-workspace="${esc(w.id)}">${agent(t.type)}<span class="grow"><strong>${esc(t.name)}</strong><small>${esc(t.type)} · ${t.running?'运行中':'休眠，点击恢复'}</small></span>${icon('chevron')}</button>`).join('')||'<p class="muted small">还没有终端会话</p>'}${btn('remote-new','新建会话','plus','outline',`data-workspace="${esc(w.id)}"`)}</section>`).join('')}</div>`:`<div class="remote-output" tabindex="0"><p class="mint flex">${agent(t.type)} ${esc(t.type)} / ${esc(state.workspaces.find(w=>w.id===remoteWorkspace)?.name)}</p><br><p class="muted">与桌面共享当前会话 · 原型演示</p><br><p>会话上下文已就绪。可以输入中文、多行消息，或使用下方的快捷键。</p><br>${(t.logs||[]).map(log=>`<p class="mint" style="white-space:pre-wrap">❯ ${esc(log.input)}</p><p style="white-space:pre-wrap">${esc(log.output)}</p><br>`).join('')}${!t.running?'<p class="muted">会话已暂停。点击下方恢复继续。</p>':''}</div><div class="remote-keys">${['Esc','Tab','Ctrl+C','↑','↓','←','→','Enter'].map(key=>`<button data-action="remote-key" data-key="${esc(key)}" ${!t.running?'disabled':''}>${key}</button>`).join('')}</div><form class="remote-compose" id="remote-compose"><textarea id="remote-message" rows="2" placeholder="输入消息…" aria-label="远程终端输入" ${!t.running?'disabled':''}></textarea><button class="primary" type="submit" aria-label="发送消息" ${!t.running?'disabled':''}>${icon('up')}</button></form>`}
  <footer class="remote-bottom"><span>原型演示 · 不连接真实终端</span>${remoteConnected&&t?btn('remote-options','','more','icon','aria-label="远程会话操作"'):remoteConnected?btn('remote-logout','断开连接',null):'<span>LAN / Tailscale</span>'}</footer></div>`;
}
function remoteDemo() { closeMenu(); openModal('远程端预览', '<div id="remote-root"></div>', '', 'remote','remote-modal'); $('.modal-body').style.padding='0'; renderRemote(); }
const actions = {
  'overview':b=>guarded(()=>{if(b?.dataset.id)state.selected=b.dataset.id;page='overview';save();render();}), 'terminal':()=>{page='terminal';render();},
  'transcript':()=>{page='transcript';render();},
  'toggle-workspace':b=>{const id=b.dataset.id,expanded=b.getAttribute('aria-expanded')!=='true';state.collapsedWorkspaces=expanded?state.collapsedWorkspaces.filter(value=>value!==id):[...state.collapsedWorkspaces,id];b.setAttribute('aria-expanded',String(expanded));b.setAttribute('aria-label',(expanded?'折叠 ':'展开 ')+(state.workspaces.find(item=>item.id===id)?.name||''));document.getElementById('workspace-sessions-'+id).hidden=!expanded;save();},
  'workspace':b=>guarded(()=>{state.selected=b.dataset.id;page=tab()?'terminal':'overview';preview=null;if(tab())tab().running=true;save();render();}),
  'add-workspace':()=>guarded(()=>workspaceDialog()),
  'workspace-menu':b=>showMenu(b,`${btn('overview','项目概览','grid','',`data-id="${esc(b.dataset.id)}"`)}${btn('rename-workspace','重命名','edit','',`data-id="${esc(b.dataset.id)}"`)}${btn('remove-workspace','移除工作区','close','danger',`data-id="${esc(b.dataset.id)}"`)}`),
  'rename-workspace':b=>workspaceDialog(state.workspaces.find(w=>w.id===b.dataset.id)),
  'remove-workspace':b=>guarded(()=>{const w=state.workspaces.find(w=>w.id===b.dataset.id); openModal('移除工作区？',`<p>移除「${esc(w.name)}」的工作区记录和会话。项目目录中的文件会保留。</p>`,`${btn('cancel-modal','取消',null)}${btn('confirm-remove','移除工作区',null,'danger outline',`data-id="${esc(w.id)}"`)}`,'remove');}),
  'confirm-remove':b=>{state.workspaces=state.workspaces.filter(w=>w.id!==b.dataset.id);if(state.selected===b.dataset.id)state.selected=state.workspaces[0]?.id;preview=null;page='overview';save();closeModal();render();toast('工作区记录已移除');},
  'new-menu':b=>newMenu(b),
  'create-tab':b=>{closeMenu();const type=b.dataset.type,t={id:crypto.randomUUID(),name:type,type,running:true,logs:[]};ws().tabs.push(t);ws().selected=t.id;page='terminal';save();render();toast(`已创建 ${type} 演示会话`);},
  'tab':b=>{const select=()=>{if(b.dataset.workspace)state.selected=b.dataset.workspace;const t=ws().tabs.find(t=>t.id===b.dataset.id);if(!t)return;const wasDormant=!t.running;if(!wasDormant&&ws().selected===t.id&&page==='terminal'&&!b.dataset.workspace)return;t.running=true;ws().selected=t.id;page='terminal';save();render();if(wasDormant)toast('已模拟恢复该标签的独立会话');};if(b.dataset.workspace&&b.dataset.workspace!==state.selected)guarded(()=>{preview=null;editing=false;select();});else select();},
  'close-tab':b=>{const t=ws().tabs.find(t=>t.id===b.dataset.id);openModal('关闭终端？',`<p>关闭「${esc(t.name)}」会结束进程并移除这个标签的显示历史。</p>`,`${btn('cancel-modal','取消',null)}${btn('confirm-close','关闭终端',null,'danger outline',`data-id="${esc(t.id)}"`)}`,'close-tab');},
  'confirm-close':b=>{const w=ws();w.tabs=w.tabs.filter(t=>t.id!==b.dataset.id);if(w.selected===b.dataset.id)w.selected=w.tabs[0]?.id;save();closeModal();render();toast('终端已关闭');},
  'session-menu':b=>showMenu(b,`${btn(tab().running?'stop':'resume',tab().running?'停止会话':'恢复会话',tab().running?'stop':'play')}${btn('restart','重新启动','refresh')}${btn('rename-tab','重命名','edit','',`data-id="${esc(tab().id)}"`)}`),
  'rename-tab':b=>{const t=ws().tabs.find(t=>t.id===(b.dataset.id||ws().selected));openModal('重命名终端',`<form id="rename-tab-form"><label>标签名称<input id="tab-name" value="${esc(t.name)}" required maxlength="64"></label></form>`,`${btn('cancel-modal','取消',null)}<button class="primary" form="rename-tab-form">保存名称</button>`,`rename-tab:${t.id}`);},
  'stop':()=>{closeMenu();openModal('停止当前会话？','<p>停止后保留标签与历史输出，可以再次恢复。正在执行的任务会中断。</p>',`${btn('cancel-modal','取消',null)}${btn('confirm-stop','停止会话',null,'danger outline')}`,'stop');},
  'confirm-stop':()=>{tab().running=false;save();closeModal();render();toast('会话已停止，历史已保留');},
  'resume':()=>{tab().running=true;closeMenu();save();render();toast('已恢复演示会话');},
  'restart':()=>{closeMenu();openModal('重新启动会话？','<p>当前进程会结束，随后按这个标签的恢复参数重新启动。</p>',`${btn('cancel-modal','取消',null)}${btn('confirm-restart','重新启动','refresh','primary')}`,'restart');},
  'confirm-restart':()=>{tab().running=true;save();closeModal();render();toast('已模拟按原标签参数重新启动');},
  'toggle-panel':()=>{rightOpen=!rightOpen;render();},
  'panel-tab':b=>{rightTab=b.dataset.id;rightOpen=true;render();},
  'explorer-folder':b=>{const name=b.dataset.folder;explorerFolders.has(name)?explorerFolders.delete(name):explorerFolders.add(name);render();},
  'explorer-file':b=>{const file=b.dataset.file;openPreview(sampleFiles[file].kind,file);},
  'scm-file':b=>{selectedChange=b.dataset.file;render();},
  'scm-stage':b=>{const change=demoChanges.find(c=>c.path===b.dataset.file);change.staged=!change.staged;render();},
  'preview-code':()=>openPreview('code'),'preview-readme':()=>openPreview('readme'),'preview-image':()=>openPreview('image'),
  'close-preview':()=>guarded(()=>{preview=null;editing=false;render();}),
  'wrap':()=>{code=$('#code-editor')?.value??code;wrap=!wrap;render();},
  'edit':()=>{if(editing)saveDraft();editing=!editing;render();$('#code-editor')?.focus();},
  'copy-preview':()=>copy(preview==='image'?`${ws().path}/assets/${imageName}`:preview==='readme'?'# VibeTerm\nA focused workspace for your coding agents.':getSelection().toString()||code),
  'copy-directory':()=>copy(ws().path),
  'settings':()=>settings(),'settings-remote':()=>settings('remote'),'shortcuts':()=>settings('shortcuts'),
  'settings-tab':b=>settings(b.dataset.id),
  'toggle-agent':b=>{const type=b.dataset.type;state.hidden=state.hidden.includes(type)?state.hidden.filter(t=>t!==type):[...state.hidden,type];save();settings('agents');},
  'scan-agents':()=>{const rows=$('#agent-rows');rows.innerHTML='<div class="skeleton"></div><div class="skeleton"></div><div class="skeleton"></div>';clearTimeout(refreshTimer);refreshTimer=setTimeout(()=>{if($('#agent-rows')){settings('agents');toast('演示检测完成，发现 4 个 Agent');}},650);},
  'toggle-web':()=>{state.web=!state.web;save();render();settings('remote');toast(state.web?'Web 访问演示已开启':'Web 访问演示已关闭');},
  'copy-lan':()=>copy('http://192.168.1.86:8787/#token=vibe-demo-token'),
  'copy-tailnet':()=>copy('http://100.101.32.16:8787/#token=vibe-demo-token'),
  'copy-token':()=>copy('vibe-demo-token'),
  'retry-tailscale':()=>toast('已重新获取演示地址：100.101.32.16'),
  'refresh-transcript':()=>{const v=$('.transcript');if(!v)return;v.innerHTML='<div class="skeleton"></div><div class="skeleton"></div><div class="skeleton"></div>';clearTimeout(refreshTimer);refreshTimer=setTimeout(()=>{if(page==='transcript')render();toast('已刷新本次演示的对话记录');},500);},
  'cancel-modal':()=>closeModal(),
  'discard':()=>{const next=pending;code=savedCode;dirty=false;editing=false;closeModal();next?.();},
  'save-continue':()=>{const next=pending;saveDraft();closeModal();next?.();},
  'remote-demo':()=>remoteDemo(),
  'fill-token':()=>{$('#remote-token').value='vibe-demo-token';$('#remote-token').focus();},
  'remote-select':b=>{remoteWorkspace=b.dataset.workspace;remoteTab=b.dataset.id;getRemoteTab().running=true;save();renderRemote();},
  'remote-list':()=>{remoteTab=null;renderRemote();},
  'remote-logout':()=>{remoteConnected=false;remoteTab=null;renderRemote();},
  'remote-new':b=>{const w=state.workspaces.find(w=>w.id===b.dataset.workspace);showMenu(b,`<div class="menu-label">新建远程会话</div>${['CMD','PowerShell',...availableAgents.filter(t=>!state.hidden.includes(t))].map(type=>btn('remote-create',type,null,'',`data-workspace="${esc(w.id)}" data-type="${type}"`)).join('')}`);},
  'remote-create':b=>{const w=state.workspaces.find(w=>w.id===b.dataset.workspace),t={id:crypto.randomUUID(),name:b.dataset.type,type:b.dataset.type,running:true,logs:[]};w.tabs.push(t);remoteWorkspace=w.id;remoteTab=t.id;closeMenu();save();renderRemote();},
  'remote-options':b=>showMenu(b,`${btn('remote-toggle',getRemoteTab().running?'停止会话':'恢复会话',getRemoteTab().running?'stop':'play')}${btn('remote-restart','重新启动','refresh')}${btn('remote-close','关闭会话','close','danger')}${btn('remote-logout','断开连接','back')}`),
  'remote-toggle':()=>{getRemoteTab().running=!getRemoteTab().running;closeMenu();save();renderRemote();},
  'remote-restart':()=>{getRemoteTab().running=true;closeMenu();save();renderRemote();toast('演示会话已重新启动');},
  'remote-close':()=>{closeMenu();const root=standaloneRemote?$('#root'):$('#remote-root');root.innerHTML=`<div class="remote-frame"><div class="empty">${icon('terminal')}<h2>关闭这个会话？</h2><p>将结束进程并移除该标签的显示历史。</p>${btn('remote-confirm-close','关闭会话',null,'danger outline')}${btn('remote-cancel-close','返回会话',null)}</div></div>`;},
  'remote-cancel-close':()=>renderRemote(),
  'remote-confirm-close':()=>{const w=state.workspaces.find(w=>w.id===remoteWorkspace);w.tabs=w.tabs.filter(t=>t.id!==remoteTab);if(w.selected===remoteTab)w.selected=w.tabs[0]?.id;remoteTab=null;save();renderRemote();},
  'remote-key':b=>{const key=b.dataset.key;if(key==='Enter'&&$('#remote-message')?.value){$('#remote-compose').requestSubmit();return;}if(key==='Ctrl+C'){getRemoteTab().logs.push({input:'^C',output:'已模拟发送中断信号。'});save();renderRemote();}else toast(`已模拟发送 ${key}`);}
};
document.addEventListener('click',event=>{
  const button=event.target.closest('[data-action]');
  if(button&&!button.disabled){actions[button.dataset.action]?.(button);if(!['new-menu','workspace-menu','session-menu','remote-options','remote-new'].includes(button.dataset.action))closeMenu();}
  else if(!event.target.closest('#menu'))closeMenu();
});
document.addEventListener('dblclick',event=>{const b=event.target.closest('[data-action=tab]');if(b)actions['rename-tab'](b);});
document.addEventListener('submit',event=>{
  event.preventDefault();
  if(event.target.id==='terminal-form')sendInput($('#command').value);
  if(event.target.id==='workspace-form'){
    const name=$('#workspace-name').value.trim(),path=$('#workspace-path')?.value.trim();
    if(!name){$('#form-error').textContent='请填写项目名称。';return;}
    if(modalOp==='add-workspace'){
      if(!path||!/^(?:[a-z]:[\\/]|\/|~\/).+/i.test(path)){$('#form-error').textContent='请填写完整目录，例如 E:/WorkSpace/my-project。';return;}
      if(state.workspaces.some(w=>w.path.toLowerCase().replace(/\\/g,'/').replace(/\/$/,'')===path.toLowerCase().replace(/\\/g,'/').replace(/\/$/,''))){$('#form-error').textContent='这个项目目录已添加，可以直接从左侧进入。';return;}
      const w={id:crypto.randomUUID(),name,path,tabs:[],selected:null};state.workspaces.push(w);state.selected=w.id;page='overview';preview=null;
    }else state.workspaces.find(w=>w.id===modalOp.split(':')[1]).name=name;
    save();closeModal();render();toast('工作区已保存');
  }
  if(event.target.id==='rename-tab-form'){
    const name=$('#tab-name').value.trim();if(!name)return;
    ws().tabs.find(t=>t.id===modalOp.split(':')[1]).name=name;save();closeModal();render();
  }
  if(event.target.id==='remote-login'){
    if($('#remote-token').value!=='vibe-demo-token'){$('#remote-error').textContent='访问令牌无效，请使用演示令牌 vibe-demo-token。';return;}
    remoteConnected=true;remoteTab=null;renderRemote();
  }
  if(event.target.id==='remote-compose'){
    const input=$('#remote-message').value;if(!input.trim())return;
    const t=getRemoteTab();if(!t?.running)return;t.logs.push({input,output:'已收到远程演示输入。桌面和远程端共享这条会话记录。'});save();renderRemote();
  }
});
document.addEventListener('input',event=>{
  if(event.target.id==='code-editor'){code=event.target.value;dirty=code!==savedCode;$('.preview-heading strong').innerHTML=`${esc(activeFile.split('/').pop())}${dirty?' <span class="mint">●</span>':''}`;$('.preview-footer>span').textContent=dirty?'未保存 · Ctrl + S 保存':'已保存至原型草稿';}
});
document.addEventListener('keydown',event=>{
  if(event.target.matches('.right-tabs [role=tab]')&&['ArrowLeft','ArrowRight','Home','End'].includes(event.key)){event.preventDefault();const index=panelTabs.findIndex(item=>item.id===rightTab);rightTab=panelTabs[event.key==='Home'?0:event.key==='End'?panelTabs.length-1:(index+(event.key==='ArrowLeft'?-1:1)+panelTabs.length)%panelTabs.length].id;render();$('#panel-tab-'+rightTab)?.focus();return;}
  if(event.key==='Escape'){
    if(!$('#menu').hidden){event.preventDefault();closeMenu();return;}
    if($('#modal').open)return;
    if(page==='transcript'){page='terminal';render();}else if(preview)actions['close-preview']();return;
  }
  if(event.target.id==='command'&&event.key==='Enter'&&!event.shiftKey&&!event.isComposing){event.preventDefault();sendInput(event.target.value);return;}
  if((event.ctrlKey||event.metaKey)&&event.key.toLowerCase()==='s'&&editing){event.preventDefault();saveDraft();render();return;}
  if($('#modal').open||/INPUT|TEXTAREA|SELECT/.test(event.target.tagName))return;
  if(event.key==='F1'||((event.ctrlKey||event.metaKey)&&event.key==='/')){event.preventDefault();settings('shortcuts');}
  if((event.ctrlKey||event.metaKey)&&event.key===','){event.preventDefault();settings();}
  if((event.ctrlKey||event.metaKey)&&event.key.toLowerCase()==='t'&&ws()){event.preventDefault();actions['create-tab']({dataset:{type:'CMD'}});}
  if((event.ctrlKey||event.metaKey)&&event.key.toLowerCase()==='w'&&tab()){event.preventDefault();actions['close-tab']({dataset:{id:tab().id}});}
  if((event.ctrlKey||event.metaKey)&&/^[1-9]$/.test(event.key)&&ws()?.tabs[+event.key-1]){event.preventDefault();actions.tab({dataset:{id:ws().tabs[+event.key-1].id}});}
  if(event.ctrlKey&&event.key==='Tab'&&ws()?.tabs.length){event.preventDefault();const tabs=ws().tabs,i=tabs.findIndex(t=>t.id===ws().selected),next=(i+(event.shiftKey?-1:1)+tabs.length)%tabs.length;actions.tab({dataset:{id:tabs[next].id}});}
  if(event.target.matches('.resize-handle')&&['ArrowLeft','ArrowRight'].includes(event.key)){event.preventDefault();const pane=$('.right-panel'),max=$('.workarea').clientWidth*.6;panelWidth=Math.max(300,Math.min(max,pane.offsetWidth+(event.key==='ArrowLeft'?24:-24)));pane.style.width=`${panelWidth}px`;}
});
$('#modal').addEventListener('cancel',event=>{event.preventDefault();closeMenu();closeModal();});
$('#modal').addEventListener('close',()=>{closeMenu();});
document.addEventListener('pointerdown',event=>{
  if(!event.target.matches('.resize-handle'))return;
  const handle=event.target,pane=$('.right-panel'),start=event.clientX,width=pane.offsetWidth,max=$('.workarea').clientWidth*.6;
  handle.setPointerCapture(event.pointerId);
  const move=e=>{panelWidth=Math.max(300,Math.min(max,width+start-e.clientX));pane.style.width=`${panelWidth}px`;};
  const end=()=>{handle.removeEventListener('pointermove',move);handle.removeEventListener('pointerup',end);handle.removeEventListener('pointercancel',end);};
  handle.addEventListener('pointermove',move);handle.addEventListener('pointerup',end);handle.addEventListener('pointercancel',end);
});
window.addEventListener('resize',closeMenu);
render();
