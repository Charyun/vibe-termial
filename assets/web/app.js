/* Web 端只发送明确的用户操作，重连时不自动重放输入。 */
"use strict";
const $ = (id) => document.getElementById(id);
let socket, token = "", ready = false, selected = null, running = false, readOnly = false;
let requestNumber = 0, pending = new Map(), workspaces = [], profiles = [], term = null;
let retryTimer, refreshTimer, retryCount = 0, manualDisconnect = false, subscribedInstance;
let subscribing = false;
const profileChoices = new Map();
const fragment = new URLSearchParams(location.hash.slice(1));
token = fragment.get("token") || sessionStorage.getItem("vibe-token") || "";
if (location.hash) history.replaceState(null, "", location.pathname + location.search);
$("token").value = token;

function notice(message = "") { $("notice").textContent = message; $("notice").hidden = !message; }
function toggleSidebar(open) { document.body.classList.toggle("sidebar-open", open); $("backdrop").hidden = !open; }
const mobileLayout = () => matchMedia("(max-width: 760px)").matches;
function setConnection(text) { $("connection").textContent = text; $("picker-connection").textContent = text; }
function showPicker() { $("picker").hidden = false; $("app").hidden = true; renderPicker(); }
function showSession() { $("picker").hidden = true; $("app").hidden = false; requestAnimationFrame(fit); }
let metricsContext = null;
function cellMetrics() {
  const cell = term?._core?._renderService?.dimensions?.css?.cell;
  if (cell && cell.width > 0) return { width: cell.width, height: cell.height };
  metricsContext ||= document.createElement("canvas").getContext("2d");
  const size = term?.options?.fontSize || (mobileLayout() ? 12 : 14);
  metricsContext.font = `${size}px Consolas, "Liberation Mono", monospace`;
  return { width: Math.ceil(metricsContext.measureText("W").width) + 1, height: Math.ceil(size * 1.4) };
}
// 移动端按可视宽度裁列，长行自动换行并可上下滚动；桌面端沿用主机行列，保持横向滚动。
function fit() {
  if (!term || !mobileLayout() || $("app").hidden) return;
  const box = $("terminal-scroll");
  if (!box.clientWidth || !box.clientHeight) return;
  const { width, height } = cellMetrics();
  const cols = Math.max(20, Math.floor((box.clientWidth - 16) / width));
  const rows = Math.max(8, Math.floor((box.clientHeight - 16) / height));
  if (term.cols !== cols || term.rows !== rows) term.resize(cols, rows);
}
function request(command) {
  if (!ready || socket?.readyState !== WebSocket.OPEN) return Promise.reject(new Error("连接已断开，请等待重连"));
  const request = ++requestNumber;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(request); reject(new Error("请求超时，请先检查会话状态再重试")); }, 12000);
    pending.set(request, { resolve, reject, timer });
    socket.send(JSON.stringify({ request, command }));
  });
}
function controls() {
  const editable = ready && selected && running && !readOnly;
  $("send").disabled = !editable;
  $("message").disabled = !editable;
  $("start").disabled = !ready || !selected || running || readOnly;
  $("stop").disabled = !editable;
  $("restart").disabled = !ready || !selected || readOnly;
  $("close").disabled = !ready || !selected || readOnly;
  $("keyboard").disabled = !editable;
  for (const button of $("keys").children) button.disabled = !editable;
}
function terminal() {
  if (term) return term;
  term = new Terminal({ cols: 100, rows: 30, fontSize: matchMedia("(max-width:760px)").matches ? 12 : 14,
    fontFamily: 'Consolas, "Liberation Mono", monospace', theme: { background: "#181818", foreground: "#dddde0", cursor: "#a8d5a2" },
    cursorBlink: true, scrollback: 2000, allowProposedApi: false, disableStdin: true });
  term.open($("terminal"));
  // 设备查询由主机侧解析器回应，Web 镜像不向进程重复发送查询响应。
  for (const prefix of [undefined, ">", "="]) {
    for (const final of ["n", "c"]) term.parser.registerCsiHandler({ final, ...(prefix ? { prefix } : {}) }, () => true);
  }
  term.parser.registerOscHandler(52, () => true);
  term.onData(data => {
    if (selected && ready && running && !readOnly) request({ op: "input", id: selected, data }).catch(e => notice(e.message));
  });
  return term;
}
function applySnapshot(message) {
  const t = terminal();
  t.reset();
  if (mobileLayout()) fit(); else t.resize(message.cols, message.rows);
  t.write(new Uint8Array(message.bytes)); $("empty").hidden = true;
  if (mobileLayout()) fit();
}
async function subscribe() {
  if (!selected) return;
  subscribing = true;
  const id = selected;
  const instance = workspaces.flatMap(w => w.tabs).find(t => t.id === id)?.instance;
  try {
    await request({ op: "subscribe", id });
    if (selected === id) subscribedInstance = instance;
  } finally { subscribing = false; }
}
async function choose(id) {
  if (selected !== id && $("message").value && !confirm("切换会话会清空尚未发送的草稿，是否继续？")) return;
  selected = id; $("message").value = ""; toggleSidebar(false); showSession();
  try { await subscribe(); render(); fit(); } catch (e) { notice(e.message); }
}
function renderPicker() {
  const list = $("picker-list"); list.replaceChildren();
  if (!workspaces.length) {
    const empty = document.createElement("p"); empty.className = "picker-empty"; empty.textContent = "请先在桌面端添加工作区。";
    list.append(empty); return;
  }
  for (const w of workspaces) {
    const group = document.createElement("section"); group.className = "picker-group";
    const heading = document.createElement("h2"); heading.textContent = w.name; group.append(heading);
    if (!w.tabs.length) {
      const empty = document.createElement("p"); empty.className = "picker-empty"; empty.textContent = "暂无会话，可在桌面端新建。";
      group.append(empty);
    }
    for (const tab of w.tabs) {
      const button = document.createElement("button");
      button.className = "picker-session" + (tab.id === selected ? " active" : "");
      button.setAttribute("aria-pressed", String(tab.id === selected));
      const top = document.createElement("span"); top.className = "top";
      const dot = document.createElement("span"); dot.className = "dot" + (tab.running ? " running" : "");
      const name = document.createElement("span"); name.className = "name"; name.textContent = tab.title;
      top.append(dot, name);
      const meta = document.createElement("small");
      meta.textContent = `${tab.profile} · ${tab.running ? "运行中" : "已停止，可恢复"}`;
      button.append(top, meta);
      button.onclick = () => choose(tab.id);
      group.append(button);
    }
    list.append(group);
  }
}
function render() {
  const list = $("workspaces"); list.replaceChildren();
  let active = null, workspaceName = "";
  for (const w of workspaces) {
    const section = document.createElement("section"); section.className = "workspace";
    const heading = document.createElement("div"); heading.className = "workspace-title";
    const title = document.createElement("strong"); title.textContent = w.name; heading.append(title);
    const select = document.createElement("select"); select.setAttribute("aria-label", `${w.name} 的新会话类型`);
    for (const p of profiles) { const option = document.createElement("option"); option.value = p.id; option.textContent = p.name; select.append(option); }
    if (profileChoices.has(w.id)) select.value = profileChoices.get(w.id);
    select.onchange = () => profileChoices.set(w.id, select.value);
    heading.append(select); section.append(heading);
    for (const tab of w.tabs) {
      const button = document.createElement("button"); button.className = "session" + (tab.id === selected ? " active" : "");
      button.setAttribute("aria-pressed", String(tab.id === selected));
      const dot = document.createElement("span"); dot.className = "dot" + (tab.running ? " running" : "");
      const name = document.createElement("span"); name.className = "name"; name.textContent = tab.title;
      const status = document.createElement("small"); status.textContent = tab.running ? "运行中" : "已停止";
      button.append(dot, name, status); button.onclick = () => choose(tab.id); section.append(button);
      if (tab.id === selected) { active = tab; workspaceName = w.name; }
    }
    const add = document.createElement("button"); add.className = "new-session"; add.textContent = "+ 新建会话"; add.disabled = !ready || readOnly;
    add.onclick = async () => {
      add.disabled = true;
      try { const result = await request({ op: "new", workspace: w.id, profile: select.value }); await refresh(); await choose(result.id); }
      catch (e) { notice(e.message); } finally { add.disabled = !ready || readOnly; }
    };
    section.append(add); list.append(section);
  }
  if (!workspaces.length) { const p = document.createElement("p"); p.textContent = "请先在桌面端添加工作区。"; list.append(p); }
  running = Boolean(active?.running);
  $("title").textContent = active?.title || "选择一个会话";
  $("subtitle").textContent = active ? `${workspaceName} · ${active.profile} · ${running ? "运行中" : "已停止，可恢复"}` : "桌面与 Web 共享同一终端进程";
  if (term) term.options.disableStdin = !ready || !running || readOnly;
  renderPicker();
  controls();
}
async function refresh() {
  const data = await request({ op: "list" });
  workspaces = data.workspaces; profiles = data.profiles; readOnly = data.read_only;
  const active = workspaces.flatMap(w => w.tabs).find(t => t.id === selected);
  if (selected && !active) { selected = null; subscribedInstance = undefined; term?.reset(); $("empty").hidden = false; }
  render();
  if (active && active.instance !== subscribedInstance && !subscribing) await subscribe();
}
async function action(op, question) {
  if (!selected || (question && !confirm(question))) return;
  try {
    notice(); await request({ op, id: selected });
    await refresh(); if (op !== "close") await subscribe();
  } catch (e) { notice(e.message); }
  document.querySelector(".more").open = false;
}
function connect() {
  clearTimeout(retryTimer); clearInterval(refreshTimer);
  const current = new WebSocket(`${location.protocol === "https:" ? "wss:" : "ws:"}//${location.host}/ws`);
  socket = current; let authenticated = false;
  current.onopen = () => current.send(JSON.stringify({ op: "auth", token }));
  current.onmessage = async event => {
    if (socket !== current) return;
    let m; try { m = JSON.parse(event.data); } catch { return; }
    if (m.type === "ready") {
      authenticated = true; ready = true; retryCount = 0; sessionStorage.setItem("vibe-token", token);
      $("login").hidden = true; setConnection("● 已连接"); notice();
      try {
        await refresh();
        if (selected) { showSession(); if (subscribedInstance === undefined) await subscribe(); }
        else showPicker();
      } catch (e) { notice(e.message); showPicker(); }
      refreshTimer = setInterval(() => refresh().catch(e => notice(e.message)), 3000);
      return;
    }
    if (m.type === "error" && !authenticated) {
      manualDisconnect = true; sessionStorage.removeItem("vibe-token");
      $("app").hidden = true; $("picker").hidden = true; $("login").hidden = false; $("login-error").textContent = m.error; return;
    }
    const p = pending.get(m.request);
    if (m.type === "snapshot") applySnapshot(m);
    if (m.type === "output" && term) term.write(new Uint8Array(m.bytes));
    if (m.type === "resize" && term && !mobileLayout()) term.resize(m.cols, m.rows);
    if (p) { clearTimeout(p.timer); pending.delete(m.request); m.type === "error" ? p.reject(new Error(m.error)) : p.resolve(m.data || m); }
    else if (m.type === "error") notice(m.error);
  };
  current.onclose = () => {
    if (socket !== current) return;
    ready = false; subscribedInstance = undefined; clearInterval(refreshTimer);
    for (const p of pending.values()) { clearTimeout(p.timer); p.reject(new Error("连接已断开；输入不会自动重发")); } pending.clear();
    setConnection("○ 连接断开"); controls(); if (term) term.options.disableStdin = true;
    if (!manualDisconnect) {
      const delay = Math.min(1000 * 2 ** retryCount++, 15000);
      notice("连接断开，正在重连。未确认的输入不会自动重发。");
      $("login-error").textContent = "无法连接，正在重试；请检查电脑、网络与 Web 服务。";
      retryTimer = setTimeout(connect, delay);
    }
  };
}
$("login-form").onsubmit = event => { event.preventDefault(); token = $("token").value.trim(); manualDisconnect = false; $("login-error").textContent = ""; connect(); };
function logout() {
  manualDisconnect = true; clearTimeout(retryTimer); clearInterval(refreshTimer); sessionStorage.removeItem("vibe-token"); token = "";
  socket?.close(); $("app").hidden = true; $("picker").hidden = true; $("login").hidden = false; $("token").value = ""; $("login-error").textContent = "";
}
$("logout").onclick = logout;
$("picker-logout").onclick = logout;
$("back-to-picker").onclick = () => { toggleSidebar(false); showPicker(); };
$("show-sidebar").onclick = () => toggleSidebar(true);
$("hide-sidebar").onclick = $("backdrop").onclick = () => toggleSidebar(false);
$("keyboard").onclick = () => terminal().focus();
$("start").onclick = () => action("start");
$("stop").onclick = () => action("stop", "停止会话将结束当前终端进程，是否继续？");
$("restart").onclick = () => action("restart", "重新启动会结束当前进程并创建新会话，是否继续？");
$("close").onclick = () => action("close", "关闭会话将结束进程并删除该标签的显示历史，是否继续？");
const keys = { escape: "\x1b", tab: "\t", interrupt: "\x03", up: "\x1b[A", down: "\x1b[B", left: "\x1b[D", right: "\x1b[C", enter: "\r" };
$("keys").onclick = event => {
  const key = event.target.closest("button")?.dataset.key;
  if (key && ready && running && selected) request({ op: "input", id: selected, data: keys[key] }).catch(e => notice(e.message));
};
$("compose").onsubmit = async event => {
  event.preventDefault(); const text = $("message").value; const id = selected;
  if (!text.trim() || !id || !running || !ready) return;
  $("send").disabled = true;
  try {
    await request({ op: "paste", id, data: text });
    await request({ op: "input", id, data: "\r" });
    if (selected === id && $("message").value === text) $("message").value = "";
    notice();
  } catch (e) { notice(e.message); } finally { controls(); }
};
$("message").onkeydown = event => {
  if (event.key === "Enter" && (event.ctrlKey || event.metaKey) && !event.isComposing) { event.preventDefault(); $("compose").requestSubmit(); }
};
function viewport() { if (window.visualViewport) $("app").style.height = `${window.visualViewport.height}px`; fit(); }
window.visualViewport?.addEventListener("resize", viewport); window.addEventListener("resize", viewport); window.addEventListener("orientationchange", () => setTimeout(fit, 150)); viewport(); controls();
if (token) connect();
