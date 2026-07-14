import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PhysicalPosition } from "@tauri-apps/api/dpi";
import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";
import "./overlay.css";

type SessionSnapshot = {
  phase: "idle" | "connecting" | "recording" | "processing" | "failed";
  hotkey_mode: "free" | "normal";
  partial: string;
  final_text: string;
  error: string | null;
};

const windowHandle = getCurrentWindow();
document.querySelector<HTMLDivElement>("#overlay")!.innerHTML = `
  <section class="capsule" data-phase="idle" role="status" aria-live="polite">
    <div class="wave" aria-hidden="true">
      <i></i><i></i><i></i><i></i><i></i><i></i><i></i>
    </div>
    <div class="copy"><strong id="overlay-status">正在聆听</strong><span id="overlay-detail">松开快捷键完成</span></div>
  </section>
`;

const capsule = document.querySelector<HTMLElement>(".capsule")!;
const waveBars = [...document.querySelectorAll<HTMLElement>(".wave i")];
const waveFactors = [0.58, 0.78, 0.94, 1, 0.9, 0.72, 0.52];
let failureTimer: number | undefined;
let renderQueue = Promise.resolve();

function renderAudioLevel(value: number) {
  const level = Math.max(0, Math.min(1, value));
  waveBars.forEach((bar, index) => {
    const scale = 0.35 + level * waveFactors[index] * 0.9;
    bar.style.transform = `scaleY(${scale})`;
  });
}

async function positionAtBottom() {
  const monitor = await currentMonitor();
  if (!monitor) return;
  const windowSize = await windowHandle.outerSize();
  const x = monitor.position.x + Math.round((monitor.size.width - windowSize.width) / 2);
  const y = monitor.position.y + monitor.size.height - windowSize.height - Math.round(64 * monitor.scaleFactor);
  await windowHandle.setPosition(new PhysicalPosition(x, y));
}

async function render(snapshot: SessionSnapshot) {
  try {
    window.clearTimeout(failureTimer);
    capsule.dataset.phase = snapshot.phase;
    capsule.setAttribute("aria-busy", String(snapshot.phase === "processing"));
    if (snapshot.phase === "idle") {
      await windowHandle.hide();
      return;
    }
    const content = {
      connecting: ["正在聆听", snapshot.hotkey_mode === "normal" ? "再次按下快捷键完成" : "松开快捷键完成"],
      recording: ["正在聆听", snapshot.partial || (snapshot.hotkey_mode === "normal" ? "再次按下快捷键完成" : "松开快捷键完成")],
      processing: ["处理中", "正在等待完整识别结果"],
      failed: ["识别失败", snapshot.error || "请在主窗口查看详情"],
    }[snapshot.phase];
    if (!content) return;
    document.querySelector("#overlay-status")!.textContent = content[0];
    document.querySelector("#overlay-detail")!.textContent = content[1];
    if (snapshot.phase === "processing") renderAudioLevel(0);
    await positionAtBottom();
    await windowHandle.show();
    if (snapshot.phase === "failed") failureTimer = window.setTimeout(() => void windowHandle.hide(), 3000);
  } catch (error) {
    console.error("[overlay] window operation failed", error);
  }
}

function scheduleRender(snapshot: SessionSnapshot) {
  renderQueue = renderQueue.then(() => render(snapshot));
}

await listen<SessionSnapshot>("session-update", (event) => scheduleRender(event.payload));
await listen<{ value: number }>("audio-level", (event) => renderAudioLevel(event.payload.value));
renderAudioLevel(0);
scheduleRender(await invoke<SessionSnapshot>("session_snapshot"));
