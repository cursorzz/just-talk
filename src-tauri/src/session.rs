use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use arboard::Clipboard;
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use uuid::Uuid;

use crate::{
    audio::AudioCapture,
    config::{AppConfig, HotkeyMode},
    media::PauseToken,
    protocol::{self, ServerMessage},
};

const WS_URL: &str = "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_nostream";
const RESOURCE_ID: &str = "volc.seedasr.sauc.duration";
const DELAYED_STOP: Duration = Duration::from_millis(150);
const FINAL_RESPONSE_GRACE: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    #[default]
    Idle,
    Connecting,
    Recording,
    Processing,
    Failed,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SessionSnapshot {
    pub phase: Phase,
    pub hotkey_mode: HotkeyMode,
    pub partial: String,
    pub final_text: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DebugEntry {
    pub timestamp_ms: u128,
    pub direction: &'static str,
    pub label: String,
    pub content: String,
}

enum Control {
    Stop,
    Cancel,
}

pub struct ActiveSession {
    id: Uuid,
    audio: AudioCapture,
    control: mpsc::UnboundedSender<Control>,
    media: Option<PauseToken>,
}

#[derive(Default)]
pub struct SessionManager {
    snapshot: Arc<Mutex<SessionSnapshot>>,
    active: Arc<Mutex<Option<ActiveSession>>>,
}

impl SessionManager {
    pub fn snapshot(&self) -> SessionSnapshot {
        self.snapshot.lock().clone()
    }

    pub fn start(&self, app: AppHandle, config: AppConfig) -> Result<(), String> {
        if config.app_id.trim().is_empty() || config.access_token.trim().is_empty() {
            return Err("请先填写 App ID 和 Access Token".into());
        }
        let phase = self.snapshot.lock().phase.clone();
        if !phase_allows_start(&phase) || self.active.lock().is_some() {
            return Ok(());
        }
        self.update(
            &app,
            SessionSnapshot {
                phase: Phase::Connecting,
                hotkey_mode: config.hotkey_mode.clone(),
                ..Default::default()
            },
        );
        let media = config
            .pause_media_during_recording
            .then(PauseToken::pause_current)
            .flatten();
        let (audio_tx, audio_rx) = mpsc::unbounded_channel();
        let audio = match AudioCapture::start(audio_tx) {
            Ok(audio) => audio,
            Err(error) => {
                if let Some(media) = media {
                    media.resume();
                }
                self.fail(&app, error.clone());
                return Err(error);
            }
        };
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let session_id = Uuid::new_v4();
        *self.active.lock() = Some(ActiveSession {
            id: session_id,
            audio,
            control: control_tx,
            media,
        });
        let state = self.snapshot.clone();
        let active = self.active.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) =
                run_recognition(app.clone(), state.clone(), config, audio_rx, control_rx).await
            {
                let value = SessionSnapshot {
                    phase: Phase::Failed,
                    error: Some(error),
                    ..state.lock().clone()
                };
                *state.lock() = value.clone();
                let _ = app.emit("session-update", value);
            }
            let session = {
                let mut active = active.lock();
                (active.as_ref().map(|session| session.id) == Some(session_id))
                    .then(|| active.take())
                    .flatten()
            };
            if let Some(session) = session {
                session.audio.stop();
                if let Some(media) = session.media {
                    media.resume();
                }
            }
        });
        Ok(())
    }

    pub fn stop(&self, app: &AppHandle) -> Result<(), String> {
        if matches!(&self.snapshot.lock().phase, Phase::Processing) {
            return Ok(());
        }
        if self.active.lock().is_none() {
            return Ok(());
        }
        let mut value = self.snapshot.lock().clone();
        value.phase = Phase::Processing;
        self.update(app, value);
        if let Some(media) = self
            .active
            .lock()
            .as_mut()
            .and_then(|session| session.media.take())
        {
            media.resume();
        }
        let active_session = self.active.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(DELAYED_STOP).await;
            let Some(active) = active_session.lock().take() else {
                return;
            };
            active.audio.stop();
            let _ = active.control.send(Control::Stop);
        });
        Ok(())
    }

    pub fn cancel(&self, app: &AppHandle) -> Result<(), String> {
        let Some(active) = self.active.lock().take() else {
            return Ok(());
        };
        active.audio.stop();
        if let Some(media) = active.media {
            media.resume();
        }
        active
            .control
            .send(Control::Cancel)
            .map_err(|_| "识别任务已经结束")?;
        self.update(app, SessionSnapshot::default());
        Ok(())
    }

    fn fail(&self, app: &AppHandle, error: String) {
        self.update(
            app,
            SessionSnapshot {
                phase: Phase::Failed,
                error: Some(error),
                ..Default::default()
            },
        );
    }

    fn update(&self, app: &AppHandle, value: SessionSnapshot) {
        *self.snapshot.lock() = value.clone();
        let _ = app.emit("session-update", value);
    }
}

async fn run_recognition(
    app: AppHandle,
    snapshot: Arc<Mutex<SessionSnapshot>>,
    config: AppConfig,
    mut audio_rx: mpsc::UnboundedReceiver<Vec<i16>>,
    mut control_rx: mpsc::UnboundedReceiver<Control>,
) -> Result<(), String> {
    let (request, connect_id) = recognition_socket_request(&config)?;
    debug_request(&app, &config, &connect_id, "开始识别");
    let (mut socket, _) = connect_async(request)
        .await
        .map_err(|e| format!("连接语音服务失败：{e}"))?;

    let request_json = recognition_request_json(&config);
    debug_emit(
        &app,
        &config,
        "request",
        "识别参数",
        pretty_json(&request_json),
    );
    socket
        .send(Message::Binary(
            protocol::full_request(&request_json, config.use_gzip)?.into(),
        ))
        .await
        .map_err(|e| e.to_string())?;
    mark_recording_if_connecting(&app, &snapshot);

    let mut pcm_buffer = Vec::<u8>::new();
    let mut committed = String::new();
    let mut last_end_time = -1;
    let mut stopped = false;
    let mut cancelled = false;
    let mut last_level_emit = Instant::now() - Duration::from_millis(100);
    let finalization_timeout = tokio::time::sleep(Duration::from_secs(24 * 60 * 60));
    tokio::pin!(finalization_timeout);

    loop {
        tokio::select! {
            Some(samples) = audio_rx.recv(), if !stopped => {
                if last_level_emit.elapsed() >= Duration::from_millis(50) {
                    let _ = app.emit("audio-level", json!({ "value": audio_level(&samples) }));
                    last_level_emit = Instant::now();
                }
                append_pcm(&mut pcm_buffer, &samples);
                while pcm_buffer.len() >= 6400 {
                    let rest = pcm_buffer.split_off(6400);
                    let chunk = std::mem::replace(&mut pcm_buffer, rest);
                    debug_emit(&app, &config, "request", "音频帧", format!("bytes={} last=false gzip={}", chunk.len(), config.use_gzip));
                    socket.send(Message::Binary(protocol::audio_request(&chunk, false, config.use_gzip)?.into())).await.map_err(|e| e.to_string())?;
                }
            }
            Some(control) = control_rx.recv() => {
                match control {
                    Control::Stop if !stopped => {
                        stopped = true;
                        while let Ok(samples) = audio_rx.try_recv() {
                            append_pcm(&mut pcm_buffer, &samples);
                        }
                        finalization_timeout.as_mut().reset(tokio::time::Instant::now() + FINAL_RESPONSE_GRACE);
                        debug_emit(&app, &config, "request", "音频结束帧", format!("bytes={} last=true gzip={}", pcm_buffer.len(), config.use_gzip));
                        socket.send(Message::Binary(protocol::audio_request(&pcm_buffer, true, config.use_gzip)?.into())).await.map_err(|e| e.to_string())?;
                        pcm_buffer.clear();
                    }
                    Control::Cancel => {
                        cancelled = true;
                        let _ = socket.close(None).await;
                        break;
                    }
                    Control::Stop => {}
                }
            }
            _ = &mut finalization_timeout, if stopped => {
                debug_emit(&app, &config, "info", "识别收尾", "等待最终响应 1500ms 后关闭连接".into());
                break;
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Binary(data))) => match protocol::parse(&data)? {
                        ServerMessage::Response { flags, json } => {
                            debug_emit(&app, &config, "response", format!("识别响应 flags={flags:#06b}"), pretty_json(&json));
                            let (new_text, partial) = protocol::recognition_text(&json, &mut last_end_time)?;
                            committed.push_str(&new_text);
                            let display = format!("{committed}{partial}");
                            {
                                let mut value = snapshot.lock();
                                value.partial = display;
                                value.final_text = committed.clone();
                                let _ = app.emit("session-update", value.clone());
                            }
                            if stopped && flags == 0b0011 { break; }
                        }
                        ServerMessage::Error(error) => {
                            debug_emit(&app, &config, "error", "服务端错误", error.clone());
                            return Err(format!("语音服务返回错误：{error}"));
                        },
                        ServerMessage::Other => {}
                    },
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(error)) => return Err(format!("语音连接中断：{error}")),
                    _ => {}
                }
            }
        }
    }

    if cancelled {
        let value = SessionSnapshot::default();
        *snapshot.lock() = value.clone();
        let _ = app.emit("session-update", value);
        return Ok(());
    }

    let displayed = snapshot.lock().partial.clone();
    let final_text = choose_final_text(&committed, &displayed);
    if !final_text.is_empty() {
        Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(final_text.clone()))
            .map_err(|e| format!("写入剪贴板失败：{e}"))?;
        if config.auto_paste
            && let Err(error) = paste_to_focused_app()
        {
            debug_emit(&app, &config, "error", "自动粘贴失败", error);
        }
    }
    let value = SessionSnapshot {
        phase: Phase::Idle,
        hotkey_mode: config.hotkey_mode,
        partial: String::new(),
        final_text,
        error: None,
    };
    *snapshot.lock() = value.clone();
    let _ = app.emit("session-update", value);
    let _ = socket.close(None).await;
    Ok(())
}

fn choose_final_text(committed: &str, displayed: &str) -> String {
    let committed = committed.trim();
    if committed.is_empty() {
        displayed.trim().to_owned()
    } else {
        committed.to_owned()
    }
}

fn append_pcm(buffer: &mut Vec<u8>, samples: &[i16]) {
    for sample in samples {
        buffer.extend_from_slice(&sample.to_le_bytes());
    }
}

fn audio_level(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mean_square = samples
        .iter()
        .map(|sample| {
            let normalized = *sample as f64 / i16::MAX as f64;
            normalized * normalized
        })
        .sum::<f64>()
        / samples.len() as f64;
    ((mean_square.sqrt() as f32 - 0.005) * 8.0).clamp(0.0, 1.0)
}

pub async fn test_connection(app: AppHandle, config: AppConfig) -> Result<String, String> {
    if config.app_id.trim().is_empty() || config.access_token.trim().is_empty() {
        return Err("请先填写 App ID 和 Access Token".into());
    }
    let (request, connect_id) = recognition_socket_request(&config)?;
    debug_request(&app, &config, &connect_id, "测试连接");
    let (mut socket, response) =
        tokio::time::timeout(Duration::from_secs(10), connect_async(request))
            .await
            .map_err(|_| "连接语音服务超时")?
            .map_err(|e| format!("WebSocket 握手失败：{e}"))?;
    debug_emit(
        &app,
        &config,
        "response",
        "WebSocket 握手",
        format!("HTTP {}", response.status()),
    );

    let body = recognition_request_json(&config);
    debug_emit(&app, &config, "request", "测试识别参数", pretty_json(&body));
    socket
        .send(Message::Binary(
            protocol::full_request(&body, config.use_gzip)?.into(),
        ))
        .await
        .map_err(|e| e.to_string())?;
    debug_emit(
        &app,
        &config,
        "request",
        "测试结束帧",
        "bytes=0 last=true".into(),
    );
    socket
        .send(Message::Binary(
            protocol::audio_request(&[], true, config.use_gzip)?.into(),
        ))
        .await
        .map_err(|e| e.to_string())?;

    let result = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(message) = socket.next().await {
            match message.map_err(|e| format!("读取测试响应失败：{e}"))? {
                Message::Binary(data) => match protocol::parse(&data)? {
                    ServerMessage::Response { flags, json } => {
                        debug_emit(
                            &app,
                            &config,
                            "response",
                            format!("测试响应 flags={flags:#06b}"),
                            pretty_json(&json),
                        );
                        return Ok("连接测试成功，鉴权和识别接口可用。".to_string());
                    }
                    ServerMessage::Error(error) => {
                        debug_emit(&app, &config, "error", "测试失败", error.clone());
                        return Err(format!("接口返回错误：{error}"));
                    }
                    ServerMessage::Other => {}
                },
                Message::Close(_) => return Err("服务端在测试完成前关闭连接".into()),
                _ => {}
            }
        }
        Err("服务端未返回测试结果".into())
    })
    .await
    .map_err(|_| "等待接口测试响应超时")?;
    let _ = socket.close(None).await;
    result
}

fn recognition_socket_request(
    config: &AppConfig,
) -> Result<(tokio_tungstenite::tungstenite::http::Request<()>, String), String> {
    let connect_id = uuid::Uuid::new_v4().to_string();
    let mut request = WS_URL.into_client_request().map_err(|e| e.to_string())?;
    let headers = request.headers_mut();
    headers.insert(
        "X-Api-App-Key",
        config.app_id.parse().map_err(|_| "App ID 包含无效字符")?,
    );
    headers.insert(
        "X-Api-Access-Key",
        config
            .access_token
            .parse()
            .map_err(|_| "Access Token 包含无效字符")?,
    );
    headers.insert("X-Api-Resource-Id", RESOURCE_ID.parse().unwrap());
    headers.insert("X-Api-Connect-Id", connect_id.parse().unwrap());
    Ok((request, connect_id))
}

fn recognition_request_json(config: &AppConfig) -> String {
    let mut payload = json!({
        "user": { "uid": "demo_uid" },
        "audio": { "format": "pcm", "rate": 16000, "bits": 16, "channel": 1, "language": config.language },
        "request": {
            "model_name": "bigmodel", "enable_itn": true, "enable_punc": config.enable_punc,
            "enable_ddc": config.enable_ddc, "enable_word": false, "res_type": "full", "nbest": 1,
            "use_vad": true
        }
    });
    let context = hotwords_context(&config.hotwords);
    if !context.is_empty() {
        payload["request"]["corpus"] = json!({ "context": context });
    }
    payload.to_string()
}

fn debug_request(app: &AppHandle, config: &AppConfig, connect_id: &str, label: &str) {
    let token = if config.access_token.len() > 8 {
        format!(
            "{}…{}",
            &config.access_token[..4],
            &config.access_token[config.access_token.len() - 4..]
        )
    } else {
        "***".into()
    };
    debug_emit(
        app,
        config,
        "request",
        label,
        format!(
            "URL: {WS_URL}\nX-Api-App-Key: {}\nX-Api-Access-Key: {token}\nX-Api-Resource-Id: {RESOURCE_ID}\nX-Api-Connect-Id: {connect_id}",
            config.app_id
        ),
    );
}

fn debug_emit(
    app: &AppHandle,
    config: &AppConfig,
    direction: &'static str,
    label: impl Into<String>,
    content: String,
) {
    if !config.debug_enabled {
        return;
    }
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let _ = app.emit(
        "debug-entry",
        DebugEntry {
            timestamp_ms,
            direction,
            label: label.into(),
            content,
        },
    );
}

fn pretty_json(raw: &str) -> String {
    serde_json::from_str::<serde_json::Value>(raw)
        .and_then(|value| serde_json::to_string_pretty(&value))
        .unwrap_or_else(|_| raw.to_string())
}

fn hotwords_context(raw: &str) -> String {
    let words: Vec<_> = raw
        .replace(',', "\n")
        .lines()
        .map(str::trim)
        .filter(|word| !word.is_empty())
        .map(|word| json!({ "word": word }))
        .collect();
    if words.is_empty() {
        String::new()
    } else {
        json!({ "hotwords": words }).to_string()
    }
}

fn mark_recording_if_connecting(app: &AppHandle, snapshot: &Arc<Mutex<SessionSnapshot>>) -> bool {
    let value = {
        let mut value = snapshot.lock();
        if !transition_to_recording(&mut value) {
            return false;
        }
        value.clone()
    };
    let _ = app.emit("session-update", value);
    true
}

fn transition_to_recording(snapshot: &mut SessionSnapshot) -> bool {
    if snapshot.phase != Phase::Connecting {
        return false;
    }
    snapshot.phase = Phase::Recording;
    snapshot.error = None;
    true
}

fn phase_allows_start(phase: &Phase) -> bool {
    matches!(phase, Phase::Idle | Phase::Failed)
}

fn paste_to_focused_app() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventTapLocation};
        // macOS virtual key code 9 is V. Quartz posting requires Accessibility permission.
        if let (Some(down), Some(up)) = (
            CGEvent::new_keyboard_event(None, 9, true),
            CGEvent::new_keyboard_event(None, 9, false),
        ) {
            CGEvent::set_flags(Some(&down), CGEventFlags::MaskCommand);
            CGEvent::set_flags(Some(&up), CGEventFlags::MaskCommand);
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&down));
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&up));
            return Ok(());
        }
        Err("无法创建系统粘贴事件".into())
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("sh")
            .args(["-c", "command -v wtype >/dev/null && wtype -M ctrl v -m ctrl || xdotool key --clearmodifiers ctrl+v"])
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("启动系统粘贴命令失败：{error}"))
    }
    #[cfg(target_os = "windows")]
    {
        use std::mem::size_of;
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_CONTROL,
            VK_V,
        };

        let keyboard = |key, flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        };
        let inputs = [
            keyboard(VK_CONTROL, Default::default()),
            keyboard(VK_V, Default::default()),
            keyboard(VK_V, KEYEVENTF_KEYUP),
            keyboard(VK_CONTROL, KEYEVENTF_KEYUP),
        ];
        let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
        if sent == inputs.len() as u32 {
            Ok(())
        } else {
            Err(format!(
                "系统只发送了 {sent}/{} 个粘贴按键事件",
                inputs.len()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Phase, SessionSnapshot, audio_level, choose_final_text, phase_allows_start,
        transition_to_recording,
    };

    #[test]
    fn finalizes_from_displayed_nostream_text_when_no_utterance_was_committed() {
        assert_eq!(choose_final_text("", "  已完成识别  "), "已完成识别");
    }

    #[test]
    fn committed_utterances_take_precedence_over_partial_text() {
        assert_eq!(choose_final_text("确定文本", "临时文本"), "确定文本");
    }

    #[test]
    fn audio_level_is_normalized_and_ignores_silence() {
        assert_eq!(audio_level(&[0; 160]), 0.0);
        assert!(audio_level(&[12_000; 160]) > 0.9);
        assert_eq!(audio_level(&[i16::MAX; 160]), 1.0);
    }

    #[test]
    fn a_connected_socket_cannot_restore_recording_after_stop() {
        let mut snapshot = SessionSnapshot {
            phase: Phase::Processing,
            ..Default::default()
        };

        let transitioned = transition_to_recording(&mut snapshot);

        assert!(!transitioned);
        assert_eq!(snapshot.phase, Phase::Processing);
    }

    #[test]
    fn processing_blocks_a_new_recognition_session() {
        assert!(!phase_allows_start(&Phase::Processing));
        assert!(phase_allows_start(&Phase::Idle));
        assert!(phase_allows_start(&Phase::Failed));
    }
}
