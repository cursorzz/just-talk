mod audio;
mod config;
mod media;
mod permissions;
mod protocol;
mod session;

use parking_lot::RwLock;
use tauri::{
    AppHandle, Manager, State,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use config::{AppConfig, HotkeyMode};
use permissions::PermissionStatus;
use session::{SessionManager, SessionSnapshot};

struct AppState {
    config: RwLock<AppConfig>,
    session: SessionManager,
}

#[tauri::command]
fn load_config(state: State<'_, AppState>) -> AppConfig {
    state.config.read().clone()
}

#[tauri::command]
fn session_snapshot(state: State<'_, AppState>) -> SessionSnapshot {
    state.session.snapshot()
}

#[tauri::command]
fn permission_status(app: AppHandle, state: State<'_, AppState>) -> PermissionStatus {
    let status = permissions::status();
    let hotkey = state.config.read().hotkey.clone();
    if status.all_granted && !app.global_shortcut().is_registered(hotkey.as_str()) {
        let _ = register_shortcut(&app, &hotkey);
    }
    status
}

#[tauri::command]
fn request_permission(kind: String) -> Result<PermissionStatus, String> {
    permissions::request(&kind)
}

#[tauri::command]
fn open_permission_settings(kind: String) -> Result<(), String> {
    permissions::open_settings(&kind)
}

#[tauri::command]
fn start_recording(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    if !permissions::status().all_granted {
        return Err("请先开启全部必需权限".into());
    }
    state.session.start(app, state.config.read().clone())
}

#[tauri::command]
fn stop_recording(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.session.stop(&app)
}

#[tauri::command]
fn cancel_recording(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.session.cancel(&app)
}

#[tauri::command]
async fn test_connection(app: AppHandle, config: AppConfig) -> Result<String, String> {
    validate_config(&config)?;
    session::test_connection(app, config).await
}

#[tauri::command]
fn set_hotkey(
    app: AppHandle,
    state: State<'_, AppState>,
    hotkey: String,
) -> Result<AppConfig, String> {
    let hotkey = hotkey.trim().to_string();
    if hotkey.is_empty() {
        return Err("快捷键不能为空".into());
    }
    if !permissions::status().all_granted {
        return Err("请先开启全部必需权限".into());
    }
    let old = state.config.read().clone();
    if old.hotkey == hotkey {
        return Ok(old);
    }

    register_shortcut(&app, &hotkey)
        .map_err(|error| format!("快捷键可能已被占用或不受支持：{error}"))?;
    if let Err(error) = app.global_shortcut().unregister(old.hotkey.as_str()) {
        let _ = app.global_shortcut().unregister(hotkey.as_str());
        return Err(format!("替换旧快捷键失败：{error}"));
    }

    let mut updated = old.clone();
    updated.hotkey = hotkey.clone();
    if let Err(error) = config::save(&updated) {
        let _ = app.global_shortcut().unregister(hotkey.as_str());
        let _ = register_shortcut(&app, &old.hotkey);
        return Err(error);
    }
    *state.config.write() = updated.clone();
    Ok(updated)
}

#[tauri::command]
fn save_config(state: State<'_, AppState>, config: AppConfig) -> Result<AppConfig, String> {
    validate_config(&config)?;
    let old_hotkey = state.config.read().hotkey.clone();
    if old_hotkey != config.hotkey && permissions::status().all_granted {
        return Err("请点击快捷键输入框并按下新组合键进行修改".into());
    }
    config::save(&config)?;
    *state.config.write() = config.clone();
    Ok(config)
}

fn validate_config(config: &AppConfig) -> Result<(), String> {
    if config.hotkey.trim().is_empty() {
        return Err("全局快捷键不能为空".into());
    }
    if !matches!(config.language.as_str(), "zh-CN" | "en-US") {
        return Err("不支持的识别语言".into());
    }
    Ok(())
}

fn register_shortcut(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let handle = app.clone();
    app.global_shortcut()
        .on_shortcut(hotkey, move |_app, _shortcut, event| {
            let state = handle.state::<AppState>();
            let phase = state.session.snapshot().phase;
            let mode = state.config.read().hotkey_mode.clone();
            match hotkey_action(&mode, &phase, event.state) {
                HotkeyAction::Start => {
                    let _ = state
                        .session
                        .start(handle.clone(), state.config.read().clone());
                }
                HotkeyAction::Stop => {
                    let _ = state.session.stop(&handle);
                }
                HotkeyAction::Ignore => {}
            }
        })
        .map_err(|e| format!("注册快捷键失败：{e}"))
}

#[derive(Debug, PartialEq, Eq)]
enum HotkeyAction {
    Start,
    Stop,
    Ignore,
}

fn hotkey_action(mode: &HotkeyMode, phase: &session::Phase, state: ShortcutState) -> HotkeyAction {
    match (mode, phase, state) {
        (_, session::Phase::Idle | session::Phase::Failed, ShortcutState::Pressed) => {
            HotkeyAction::Start
        }
        (
            HotkeyMode::Normal,
            session::Phase::Connecting | session::Phase::Recording,
            ShortcutState::Pressed,
        )
        | (
            HotkeyMode::Free,
            session::Phase::Connecting | session::Phase::Recording,
            ShortcutState::Released,
        ) => HotkeyAction::Stop,
        _ => HotkeyAction::Ignore,
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            config: RwLock::new(config::load()),
            session: SessionManager::default(),
        })
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            session_snapshot,
            permission_status,
            request_permission,
            open_permission_settings,
            start_recording,
            stop_recording,
            cancel_recording,
            test_connection,
            set_hotkey
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let hotkey = app.state::<AppState>().config.read().hotkey.clone();
            if permissions::status().all_granted {
                register_shortcut(&handle, &hotkey)?;
            }

            let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new().menu(&menu).tooltip("JustTalk");
            #[cfg(target_os = "macos")]
            {
                let icon =
                    tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?;
                tray = tray.icon(icon).icon_as_template(true);
            }
            #[cfg(not(target_os = "macos"))]
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, event| match event.id.as_ref() {
                "show" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "quit" => app.exit(0),
                _ => {}
            })
            .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run JustTalk");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_mode_holds_to_record() {
        assert_eq!(
            hotkey_action(
                &HotkeyMode::Free,
                &session::Phase::Idle,
                ShortcutState::Pressed,
            ),
            HotkeyAction::Start
        );
        assert_eq!(
            hotkey_action(
                &HotkeyMode::Free,
                &session::Phase::Recording,
                ShortcutState::Released,
            ),
            HotkeyAction::Stop
        );
    }

    #[test]
    fn normal_mode_toggles_on_press_and_ignores_release() {
        assert_eq!(
            hotkey_action(
                &HotkeyMode::Normal,
                &session::Phase::Idle,
                ShortcutState::Pressed,
            ),
            HotkeyAction::Start
        );
        assert_eq!(
            hotkey_action(
                &HotkeyMode::Normal,
                &session::Phase::Recording,
                ShortcutState::Released,
            ),
            HotkeyAction::Ignore
        );
        assert_eq!(
            hotkey_action(
                &HotkeyMode::Normal,
                &session::Phase::Recording,
                ShortcutState::Pressed,
            ),
            HotkeyAction::Stop
        );
    }
}
