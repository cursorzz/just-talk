import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import "./styles.css";

type AppConfig = {
  app_id: string;
  access_token: string;
  language: string;
  hotkey: string;
  hotkey_mode: "free" | "normal";
  auto_paste: boolean;
  use_gzip: boolean;
  enable_punc: boolean;
  enable_ddc: boolean;
  hotwords: string;
  debug_enabled: boolean;
  pause_media_during_recording: boolean;
};

type DebugEntry = {
  timestamp_ms: number;
  direction: "request" | "response" | "error" | "info";
  label: string;
  content: string;
};

type SessionSnapshot = {
  phase: "idle" | "connecting" | "recording" | "processing" | "failed";
  hotkey_mode: "free" | "normal";
  partial: string;
  final_text: string;
  error: string | null;
};

type PermissionStatus = {
  required: boolean;
  microphone: boolean;
  accessibility: boolean;
  all_granted: boolean;
};

const defaults: AppConfig = {
  app_id: "",
  access_token: "",
  language: "zh-CN",
  hotkey: "CommandOrControl+Shift+Space",
  hotkey_mode: "free",
  auto_paste: true,
  use_gzip: false,
  enable_punc: true,
  enable_ddc: false,
  hotwords: "",
  debug_enabled: false,
  pause_media_during_recording: true,
};

document.querySelector<HTMLDivElement>("#app")!.innerHTML = `
  <section class="permission-gate" id="permission-gate" hidden aria-labelledby="permission-title">
    <div class="permission-panel">
      <p class="eyebrow">MACOS PERMISSIONS</p>
      <h1 id="permission-title">开启全部权限后开始使用</h1>
      <p class="lede">JustTalk 需要以下两项系统权限才能提供完整的全局语音输入。权限只用于本机录音和输入识别结果。</p>
      <div class="permission-list">
        <article><div><h2>麦克风</h2><p>采集你的语音并发送到已配置的识别服务。</p></div><span id="perm-microphone">未开启</span><button type="button" data-permission="microphone">开启</button></article>
        <article><div><h2>辅助功能</h2><p>把识别结果粘贴到当前正在使用的应用。</p></div><span id="perm-accessibility">未开启</span><button type="button" data-permission="accessibility">开启</button></article>
      </div>
      <p class="permission-help">如果辅助功能已经开启但仍显示未开启，请在系统设置中删除旧条目，重新添加“应用程序”文件夹中的当前版本，然后重新启动应用。</p>
      <div class="permission-actions"><button class="primary" id="recheck-permissions" type="button">重新检测</button><p id="permission-message" role="status" aria-live="polite"></p></div>
    </div>
  </section>
  <main class="shell">
    <section class="result-card" aria-labelledby="result-title">
      <div class="section-heading">
        <div>
          <h2 id="result-title">本次识别</h2>
          <p>识别中的文本会实时出现在这里。</p>
        </div>
        <div class="record-actions">
          <div class="status" id="status" data-phase="idle" role="status" aria-live="polite">
            <span class="status-dot" aria-hidden="true"></span>
            <span id="status-label">就绪</span>
          </div>
          <button class="primary" id="toggle" type="button">开始录音</button>
        </div>
      </div>
      <div class="transcript empty" id="transcript">还没有识别内容</div>
      <p class="inline-error" id="session-error" role="alert"></p>
    </section>

    <form class="settings-stack" id="settings-form">
      <section class="input-card" aria-labelledby="input-title">
        <h2 id="input-title">输入方式</h2>
        <fieldset class="mode-group">
          <legend>快捷键模式</legend>
          <div class="mode-options">
            <label class="mode-option">
              <input id="hotkey-mode-free" name="hotkey-mode" type="radio" value="free" />
              <span class="mode-copy"><strong>自由模式 <small>默认</small></strong><span>按住录音，松开停止</span></span>
            </label>
            <label class="mode-option">
              <input id="hotkey-mode-normal" name="hotkey-mode" type="radio" value="normal" />
              <span class="mode-copy"><strong>常规模式</strong><span>按一次开始，再按一次停止</span></span>
            </label>
          </div>
        </fieldset>
        <div class="shortcut-row">
          <div>
            <span class="field-label">全局快捷键</span>
            <div class="shortcut-display" id="hotkey-display" aria-live="polite"></div>
          </div>
          <button class="secondary" id="change-hotkey" type="button" aria-describedby="hotkey-status">更改快捷键</button>
        </div>
        <p class="field-status" id="hotkey-status" role="status" aria-live="polite"></p>
        <p class="field-status auto-save-status" id="behavior-status" role="status" aria-live="polite"></p>
      </section>

      <section class="service-card" aria-labelledby="service-title">
        <div class="service-heading">
          <div>
            <h2 id="service-title">语音服务</h2>
            <p>使用火山引擎「豆包流式语音识别模型 2.0（小时版）」</p>
          </div>
          <nav class="service-links" aria-label="火山引擎语音服务相关链接">
            <a href="https://www.volcengine.com/docs/6561/1354869?lang=zh" data-external>API 文档</a>
            <a href="https://console.volcengine.com/speech/new/" data-external>开通服务</a>
          </nav>
        </div>
        <p class="service-note">请先在火山引擎控制台开通该模型，再填写 App ID 与 Access Token。录音音频直接发送至火山引擎 API，JustTalk 不在本地保存录音。</p>
        <div class="service-grid">
          <label><span>App ID</span><input id="app-id" autocomplete="off" required /></label>
          <label><span>Access Token</span><input id="access-token" type="password" autocomplete="off" required /></label>
          <button class="secondary" id="test-connection" type="button">测试连接</button>
          <button class="primary" id="save-settings" type="submit">保存服务</button>
        </div>
        <div class="form-feedback">
          <p class="test-status" id="test-status" role="status" aria-live="polite"></p>
          <p class="save-status" id="save-status" role="status" aria-live="polite"></p>
        </div>
      </section>

      <details class="advanced-card" id="advanced-settings">
        <summary><span>高级设置</span></summary>
        <div class="advanced-content">
          <div class="advanced-grid">
            <label><span>识别语言</span><select id="language"><option value="zh-CN">中文</option><option value="en-US">English</option></select></label>
            <label class="wide"><span>热词（每行一个）</span><textarea id="hotwords" rows="3"></textarea></label>
          </div>
          <div class="switches">
            <label><input id="auto-paste" type="checkbox" /><span>识别完成后自动粘贴</span></label>
            <label><input id="enable-punc" type="checkbox" /><span>自动标点</span></label>
            <label><input id="enable-ddc" type="checkbox" /><span>语义顺滑</span></label>
            <label><input id="use-gzip" type="checkbox" /><span>压缩传输</span></label>
            <label><input id="debug-enabled" type="checkbox" /><span>调试模式</span></label>
            <label><input id="pause-media" type="checkbox" /><span>录音时暂停媒体</span></label>
          </div>
          <p class="field-status auto-save-status" id="advanced-status" role="status" aria-live="polite"></p>
        </div>
      </details>
    </form>

    <section class="debug-card" id="debug-panel" hidden aria-labelledby="debug-title">
      <div class="section-heading">
        <div><h2 id="debug-title">接口调试</h2><p>显示请求、音频帧元数据和服务端原始响应；Access Token 已脱敏。</p></div>
        <button class="secondary" id="clear-debug" type="button">清空日志</button>
      </div>
      <div class="debug-log" id="debug-log" role="log" aria-live="polite"><p class="debug-empty">暂无接口日志，请测试连接或开始录音。</p></div>
    </section>
  </main>
`;

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const input = (id: string) => $<HTMLInputElement>(id);
const select = (id: string) => $<HTMLSelectElement>(id);
const textarea = (id: string) => $<HTMLTextAreaElement>(id);
const hasTauriBridge = "__TAURI_INTERNALS__" in window;
let config = defaults;
let snapshot: SessionSnapshot = { phase: "idle", hotkey_mode: "free", partial: "", final_text: "", error: null };
let capturingHotkey = false;
let previousHotkey = "";
let autoSaveQueue = Promise.resolve();
let hotwordsSaveTimer: number | undefined;
const autoSaveClearTimers = new Map<string, number>();

document.querySelectorAll<HTMLAnchorElement>("a[data-external]").forEach((link) => {
  link.addEventListener("click", async (event) => {
    event.preventDefault();
    if (hasTauriBridge) await openUrl(link.href);
    else window.open(link.href, "_blank", "noopener,noreferrer");
  });
});

async function refreshPermissions(): Promise<boolean> {
  const status = await invoke<PermissionStatus>("permission_status");
  const gate = $("permission-gate");
  const blocked = status.required && !status.all_granted;
  gate.toggleAttribute("hidden", !blocked);
  document.querySelector("main")?.toggleAttribute("inert", blocked);
  for (const key of ["microphone", "accessibility"] as const) {
    const label = $(`perm-${key}`);
    label.textContent = status[key] ? "已开启" : "未开启";
    label.classList.toggle("granted", status[key]);
  }
  return status.all_granted;
}

document.querySelectorAll<HTMLButtonElement>("[data-permission]").forEach((button) => {
  button.addEventListener("click", async () => {
    const kind = button.dataset.permission!;
    const message = $("permission-message");
    message.textContent = "请在系统提示或系统设置中允许此权限。";
    try {
      await invoke("request_permission", { kind });
      await invoke("open_permission_settings", { kind });
      window.setTimeout(() => void refreshPermissions(), 800);
    } catch (error) { message.textContent = String(error); }
  });
});

$("recheck-permissions").addEventListener("click", () => void refreshPermissions());
window.addEventListener("focus", () => void refreshPermissions());

function immediateConfigFromForm(): AppConfig {
  return {
    ...config,
    language: select("language").value,
    hotkey: config.hotkey,
    hotkey_mode: input("hotkey-mode-normal").checked ? "normal" : "free",
    auto_paste: input("auto-paste").checked,
    use_gzip: input("use-gzip").checked,
    enable_punc: input("enable-punc").checked,
    enable_ddc: input("enable-ddc").checked,
    hotwords: textarea("hotwords").value,
    debug_enabled: input("debug-enabled").checked,
    pause_media_during_recording: input("pause-media").checked,
  };
}

function configFromForm(): AppConfig {
  return {
    ...immediateConfigFromForm(),
    app_id: input("app-id").value.trim(),
    access_token: input("access-token").value.trim(),
  };
}

function queueImmediateSave(statusId: "behavior-status" | "advanced-status") {
  const status = $(statusId);
  window.clearTimeout(autoSaveClearTimers.get(statusId));
  status.textContent = "正在自动保存…";
  if (!hasTauriBridge) {
    config = immediateConfigFromForm();
    status.textContent = "已自动保存";
    return;
  }
  autoSaveQueue = autoSaveQueue.then(async () => {
    try {
      config = await invoke<AppConfig>("save_config", { config: immediateConfigFromForm() });
      status.textContent = "已自动保存";
      status.classList.remove("failed");
      const timer = window.setTimeout(() => { status.textContent = ""; }, 1600);
      autoSaveClearTimers.set(statusId, timer);
    } catch (error) {
      status.textContent = String(error);
      status.classList.add("failed");
    }
  });
}

function fillForm(value: AppConfig) {
  input("app-id").value = value.app_id;
  input("access-token").value = value.access_token;
  select("language").value = value.language;
  renderHotkey(value.hotkey);
  input("hotkey-mode-free").checked = value.hotkey_mode === "free";
  input("hotkey-mode-normal").checked = value.hotkey_mode === "normal";
  input("auto-paste").checked = value.auto_paste;
  input("use-gzip").checked = value.use_gzip;
  input("enable-punc").checked = value.enable_punc;
  input("enable-ddc").checked = value.enable_ddc;
  textarea("hotwords").value = value.hotwords;
  input("debug-enabled").checked = value.debug_enabled;
  input("pause-media").checked = value.pause_media_during_recording;
  $("debug-panel").toggleAttribute("hidden", !value.debug_enabled);
}

function hotkeyParts(value: string): string[] {
  return value.split("+").map((part) => ({
    CommandOrControl: navigator.platform.includes("Mac") ? "⌘" : "Ctrl",
    Control: navigator.platform.includes("Mac") ? "⌃" : "Ctrl",
    Alt: navigator.platform.includes("Mac") ? "⌥" : "Alt",
    Shift: navigator.platform.includes("Mac") ? "⇧" : "Shift",
  })[part] || part);
}

function renderHotkey(value: string) {
  const display = $("hotkey-display");
  display.replaceChildren();
  hotkeyParts(value).forEach((part, index) => {
    if (index) display.append(document.createTextNode("+"));
    const key = document.createElement("kbd");
    key.textContent = part;
    display.append(key);
  });
}

function capturedKey(event: KeyboardEvent): string | null {
  const modifierOnly = ["Meta", "Control", "Shift", "Alt"].includes(event.key);
  if (modifierOnly) return null;
  const modifiers: string[] = [];
  if (event.metaKey) modifiers.push("CommandOrControl");
  if (event.ctrlKey) modifiers.push("Control");
  if (event.altKey) modifiers.push("Alt");
  if (event.shiftKey) modifiers.push("Shift");
  let key = event.key;
  if (event.code === "Space") key = "Space";
  else if (/^Key[A-Z]$/.test(event.code)) key = event.code.slice(3);
  else if (/^Digit[0-9]$/.test(event.code)) key = event.code.slice(5);
  else if (/^F([1-9]|1[0-2])$/.test(event.code)) key = event.code;
  else if (key.length === 1) key = key.toUpperCase();
  const functionKey = /^F([1-9]|1[0-2])$/.test(key);
  if (!modifiers.length && !functionKey) return "";
  return [...modifiers, key].join("+");
}

function beginHotkeyCapture() {
  if (capturingHotkey) return;
  capturingHotkey = true;
  previousHotkey = config.hotkey;
  $("hotkey-display").textContent = "请按下新的快捷键…";
  $("change-hotkey").classList.add("capturing");
  $("hotkey-status").textContent = "正在捕获，按 Esc 取消。";
  $("change-hotkey").focus();
}

$("change-hotkey").addEventListener("click", beginHotkeyCapture);

window.addEventListener("keydown", async (event) => {
  if (!capturingHotkey) return;
  event.preventDefault();
  event.stopPropagation();
  if (event.key === "Escape") {
    capturingHotkey = false;
    renderHotkey(previousHotkey);
    $("change-hotkey").classList.remove("capturing");
    $("hotkey-status").textContent = "已取消修改。";
    return;
  }
  const hotkey = capturedKey(event);
  if (hotkey === null) return;
  if (!hotkey) {
    $("hotkey-status").textContent = "请同时按下 Command、Control、Option 或 Shift；F1–F12 可单独使用。";
    return;
  }
  capturingHotkey = false;
  $("change-hotkey").classList.remove("capturing");
  $("hotkey-status").textContent = "正在检查快捷键…";
  try {
    await autoSaveQueue;
    config = await invoke<AppConfig>("set_hotkey", { hotkey });
    renderHotkey(config.hotkey);
    $("hotkey-status").textContent = "快捷键已启用并保存。";
    $("hotkey-status").classList.remove("failed");
  } catch (error) {
    renderHotkey(previousHotkey);
    $("hotkey-status").textContent = String(error);
    $("hotkey-status").classList.add("failed");
  }
}, true);

function appendDebug(entry: DebugEntry) {
  if (!input("debug-enabled").checked) return;
  const log = $("debug-log");
  log.querySelector(".debug-empty")?.remove();
  const item = document.createElement("article");
  item.className = `debug-entry ${entry.direction}`;
  const time = new Date(entry.timestamp_ms).toLocaleTimeString("zh-CN", { hour12: false });
  item.innerHTML = `<header><time>${time}</time><strong></strong><span></span></header><pre></pre>`;
  item.querySelector("strong")!.textContent = entry.direction.toUpperCase();
  item.querySelector("span")!.textContent = entry.label;
  item.querySelector("pre")!.textContent = entry.content;
  log.append(item);
  while (log.children.length > 300) log.firstElementChild?.remove();
  log.scrollTop = log.scrollHeight;
}

function renderSession(value: SessionSnapshot) {
  snapshot = value;
  const labels = { idle: "就绪", connecting: "正在聆听", recording: "正在聆听", processing: "正在整理", failed: "出现错误" };
  const status = $("status");
  status.dataset.phase = value.phase;
  $("status-label").textContent = labels[value.phase];
  $("toggle").textContent = ["connecting", "recording"].includes(value.phase) ? "结束录音" : "开始录音";
  $("toggle").toggleAttribute("disabled", value.phase === "processing");
  const text = value.partial || value.final_text;
  const transcript = $("transcript");
  transcript.textContent = text || "还没有识别内容";
  transcript.classList.toggle("empty", !text);
  $("session-error").textContent = value.error || "";
}

$("settings-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const status = $("save-status");
  status.textContent = "正在保存…";
  try {
    await autoSaveQueue;
    config = await invoke<AppConfig>("save_config", { config: configFromForm() });
    fillForm(config);
    status.textContent = "语音服务设置已保存。";
  } catch (error) {
    status.textContent = String(error);
  }
});

$("toggle").addEventListener("click", async () => {
  try {
    if (["connecting", "recording"].includes(snapshot.phase)) await invoke("stop_recording");
    else await invoke("start_recording");
  } catch (error) {
    renderSession({ ...snapshot, phase: "failed", error: String(error) });
  }
});

document.querySelectorAll<HTMLInputElement>('input[name="hotkey-mode"]').forEach((control) => {
  control.addEventListener("change", () => queueImmediateSave("behavior-status"));
});

document.querySelectorAll<HTMLInputElement | HTMLSelectElement>("#advanced-settings input, #advanced-settings select").forEach((control) => {
  control.addEventListener("change", () => {
    if (control.id === "debug-enabled") {
      $("debug-panel").toggleAttribute("hidden", !input("debug-enabled").checked);
    }
    queueImmediateSave("advanced-status");
  });
});

textarea("hotwords").addEventListener("input", () => {
  window.clearTimeout(hotwordsSaveTimer);
  $("advanced-status").textContent = "等待自动保存…";
  hotwordsSaveTimer = window.setTimeout(() => queueImmediateSave("advanced-status"), 350);
});

$("test-connection").addEventListener("click", async () => {
  const button = $<HTMLButtonElement>("test-connection");
  const status = $("test-status");
  button.disabled = true;
  status.textContent = "正在调用语音识别接口…";
  try {
    const message = await invoke<string>("test_connection", { config: configFromForm() });
    status.textContent = message;
    status.classList.remove("failed");
  } catch (error) {
    status.textContent = String(error);
    status.classList.add("failed");
  } finally { button.disabled = false; }
});

$("clear-debug").addEventListener("click", () => {
  $("debug-log").innerHTML = '<p class="debug-empty">暂无接口日志，请测试连接或开始录音。</p>';
});

async function bootstrap() {
  await refreshPermissions();
  config = await invoke<AppConfig>("load_config");
  fillForm(config);
  renderSession(await invoke<SessionSnapshot>("session_snapshot"));
  await listen<SessionSnapshot>("session-update", (event) => renderSession(event.payload));
  await listen<DebugEntry>("debug-entry", (event) => appendDebug(event.payload));
}

if (hasTauriBridge) {
  bootstrap().catch((error) => renderSession({ ...snapshot, phase: "failed", error: String(error) }));
} else {
  fillForm({ ...defaults, app_id: "6574226278", access_token: "preview-token-for-layout" });
  renderSession({ ...snapshot, final_text: "我觉得常规模式还挺有用的。常规模式下面呢，有点意思。" });
}
