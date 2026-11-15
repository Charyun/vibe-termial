// 浏览器验收只连接 tests/web.rs 创建的隔离会话夹具。
const fs = require("node:fs");
const path = require("node:path");
const assert = require("node:assert/strict");
const { chromium } = require(require.resolve("playwright", { paths: [process.cwd(), path.join(process.cwd(), ".local/web-check")] }));
const endpointFile = process.env.VIBE_WEB_FIXTURE_FILE;
if (!endpointFile) throw new Error("请指定 VIBE_WEB_FIXTURE_FILE，禁止将此验收脚本连接到真实用户会话");
const endpoint = JSON.parse(fs.readFileSync(endpointFile, "utf8"));
const output = path.join(process.cwd(), ".reference");
fs.mkdirSync(output, { recursive: true });

async function hasOutput(page, marker) {
  await page.waitForFunction(marker => {
    if (typeof term === "undefined" || !term) return false;
    const b = term.buffer.active;
    for (let i = 0; i < b.length; i++) if (b.getLine(i)?.translateToString().includes(marker)) return true;
    return false;
  }, marker, { timeout: 20000 });
}
async function compose(page, suffix) {
  const command = process.platform === "win32"
    ? `[Console]::WriteLine(('BROWSER_' + '${suffix}'))`
    : `printf 'BROWSER_%s\\n' '${suffix}'`;
  await page.locator("#message").fill(command);
  await page.getByRole("button", { name: "发送", exact: true }).click();
  await hasOutput(page, `BROWSER_${suffix}`);
}
async function noPageOverflow(page) {
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), "页面不应发生横向溢出");
}

(async () => {
  const browser = await chromium.launch({ channel: "msedge", headless: true });
  const errors = [];
  const watch = page => { page.on("pageerror", e => errors.push(e.message)); };
  try {
    const desktop = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await desktop.newPage(); watch(page);
    await page.goto(endpoint.url);
    await page.locator("#app").waitFor({ state: "visible" });
    assert.equal(new URL(page.url()).hash, "", "访问令牌应从地址栏移除");
    await page.locator(".session").filter({ hasText: "运行中" }).first().click();
    await page.locator("#terminal .xterm").waitFor();
    await compose(page, "桌面验收");
    await noPageOverflow(page);
    await page.screenshot({ path: path.join(output, "web-desktop.png") });

    const mobileContext = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, deviceScaleFactor: 2 });
    const mobile = await mobileContext.newPage(); watch(mobile);
    await mobile.goto(endpoint.url);
    await mobile.locator(".session").filter({ hasText: "运行中" }).first().click();
    await hasOutput(mobile, "BROWSER_桌面验收");
    assert(!(await mobile.locator("body").getAttribute("class") || "").includes("sidebar-open"));
    await compose(mobile, "手机中文");
    await hasOutput(page, "BROWSER_手机中文");
    await noPageOverflow(mobile);
    await mobile.screenshot({ path: path.join(output, "web-mobile.png") });
    for (const [width, height, name] of [[320, 568, "small"], [844, 390, "landscape"], [390, 400, "keyboard"]]) {
      await mobile.setViewportSize({ width, height });
      await mobile.waitForFunction(() => Math.abs(document.getElementById("app").getBoundingClientRect().height - window.visualViewport.height) < 2);
      await noPageOverflow(mobile);
      const composeBox = await mobile.locator("#compose").boundingBox();
      await mobile.screenshot({ path: path.join(output, `web-mobile-${name}.png`), animations: "disabled" });
      assert(composeBox.y + composeBox.height <= height + 1, `输入区应保持在可见区域内：${name} ${JSON.stringify(composeBox)}`);
    }
    await mobile.setViewportSize({ width: 390, height: 844 });
    await mobile.getByRole("button", { name: "打开会话列表" }).click();
    await mobile.locator("body.sidebar-open").waitFor();
    await mobile.getByRole("button", { name: "关闭会话列表", exact: true }).first().click();
    page.once("dialog", dialog => dialog.accept());
    await page.getByRole("button", { name: "停止", exact: true }).click();
    await page.getByRole("button", { name: "恢复", exact: true }).waitFor({ state: "visible" });
    await page.waitForFunction(() => !document.getElementById("start").disabled);
    await page.getByRole("button", { name: "恢复", exact: true }).click();
    await mobile.waitForFunction(() => !document.getElementById("send").disabled);
    // 等待另一客户端通过会话代次变化重新订阅新 PTY。
    await mobile.waitForFunction(() => typeof subscribedInstance === "string" && subscribedInstance === workspaces.flatMap(w => w.tabs).find(t => t.id === selected)?.instance);
    await compose(mobile, "恢复后同步");
    await hasOutput(page, "BROWSER_恢复后同步");

    const unauthorized = await browser.newContext();
    const bad = await unauthorized.newPage(); watch(bad);
    const base = new URL(endpoint.url).origin;
    await bad.goto(base + "/#token=invalid");
    await bad.locator("#login-error").filter({ hasText: "访问令牌无效" }).waitFor();
    assert(await bad.locator("#app").isHidden());
    const response = await page.request.get(base + "/");
    assert.equal(response.headers()["referrer-policy"], "no-referrer");
    assert(response.headers()["content-security-policy"].includes("frame-ancestors 'none'"));
    const tailnet = process.env.VIBE_WEB_TAILSCALE_IP;
    if (tailnet) {
      const tailPage = await unauthorized.newPage(); watch(tailPage);
      await tailPage.goto(`http://${tailnet}:${endpoint.port}/#token=${new URL(endpoint.url).hash.split("=")[1]}`);
      await tailPage.locator("#app").waitFor({ state: "visible" });
      console.log("已通过本机 Tailscale IPv4 完成 HTTP 与 WebSocket 鉴权");
    }
    assert.deepEqual(errors, [], "浏览器不应出现脚本异常");
    console.log("桌面、手机竖屏/小屏/横屏/键盘高度、中文输入、双客户端同步、恢复后订阅、鉴权检查全部通过");
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
