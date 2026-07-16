use tauri::AppHandle;
use tauri_plugin_global_shortcut::GlobalShortcutExt;
#[cfg(target_os = "linux")]
use tauri_plugin_global_shortcut::ShortcutState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(target_os = "linux", test))]
pub enum LinuxBackend {
    WaylandPortal,
    X11,
}

#[cfg(any(target_os = "linux", test))]
pub fn linux_backend(session_type: Option<&str>, wayland_display: Option<&str>) -> LinuxBackend {
    if session_type.is_some_and(|value| value.eq_ignore_ascii_case("wayland"))
        && wayland_display.is_some_and(|value| !value.trim().is_empty())
    {
        LinuxBackend::WaylandPortal
    } else {
        LinuxBackend::X11
    }
}

#[cfg(target_os = "linux")]
fn current_linux_backend() -> LinuxBackend {
    linux_backend(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
    )
}

#[cfg(target_os = "linux")]
struct PortalRegistration {
    hotkey: String,
    cancel: tokio::sync::oneshot::Sender<()>,
}

#[derive(Default)]
pub struct HotkeyManager {
    operation: tokio::sync::Mutex<()>,
    #[cfg(target_os = "linux")]
    portal: tokio::sync::Mutex<Option<PortalRegistration>>,
}

impl HotkeyManager {
    pub async fn is_registered(&self, app: &AppHandle, hotkey: &str) -> bool {
        #[cfg(target_os = "linux")]
        if current_linux_backend() == LinuxBackend::WaylandPortal {
            return self
                .portal
                .lock()
                .await
                .as_ref()
                .is_some_and(|registration| registration.hotkey == hotkey);
        }

        app.global_shortcut().is_registered(hotkey)
    }

    pub async fn register(&self, app: &AppHandle, hotkey: &str) -> Result<(), String> {
        let _operation = self.operation.lock().await;
        #[cfg(target_os = "linux")]
        if current_linux_backend() == LinuxBackend::WaylandPortal {
            if self
                .portal
                .lock()
                .await
                .as_ref()
                .is_some_and(|registration| registration.hotkey == hotkey)
            {
                return Ok(());
            }
            let registration = register_portal_shortcut(app.clone(), hotkey).await?;
            let old = self.portal.lock().await.replace(registration);
            if let Some(old) = old {
                let _ = old.cancel.send(());
            }
            return Ok(());
        }

        if app.global_shortcut().is_registered(hotkey) {
            Ok(())
        } else {
            register_tauri_shortcut(app, hotkey)
        }
    }

    pub async fn replace(
        &self,
        app: &AppHandle,
        old_hotkey: &str,
        new_hotkey: &str,
    ) -> Result<(), String> {
        let _operation = self.operation.lock().await;
        #[cfg(target_os = "linux")]
        if current_linux_backend() == LinuxBackend::WaylandPortal {
            let registration = register_portal_shortcut(app.clone(), new_hotkey).await?;
            let old = self.portal.lock().await.replace(registration);
            if let Some(old) = old {
                let _ = old.cancel.send(());
            }
            return Ok(());
        }

        register_tauri_shortcut(app, new_hotkey)?;
        if let Err(error) = app.global_shortcut().unregister(old_hotkey) {
            let _ = app.global_shortcut().unregister(new_hotkey);
            return Err(format!("替换旧快捷键失败：{error}"));
        }
        Ok(())
    }
}

fn register_tauri_shortcut(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let handle = app.clone();
    app.global_shortcut()
        .on_shortcut(hotkey, move |_app, _shortcut, event| {
            crate::dispatch_hotkey_event(&handle, event.state);
        })
        .map_err(|error| format!("注册快捷键失败：{error}"))
}

#[cfg(target_os = "linux")]
async fn register_portal_shortcut(
    app: AppHandle,
    hotkey: &str,
) -> Result<PortalRegistration, String> {
    use ashpd::{
        AppID,
        desktop::{
            CreateSessionOptions,
            global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut},
        },
        register_host_app_with_connection,
    };
    use futures_util::StreamExt;

    const ACTION_ID: &str = "record";
    let trigger = tauri_hotkey_to_xdg(hotkey)?;
    let connection = ashpd::zbus::Connection::session()
        .await
        .map_err(|error| format!("无法连接桌面快捷键服务：{error}"))?;
    let app_id =
        AppID::try_from("com.justtalk.slim").map_err(|error| format!("应用标识无效：{error}"))?;
    register_host_app_with_connection(connection.clone(), app_id)
        .await
        .map_err(|error| format!("无法向桌面快捷键服务注册 JustTalk：{error}"))?;

    let portal = GlobalShortcuts::with_connection(connection)
        .await
        .map_err(|error| format!("当前桌面不支持 GlobalShortcuts Portal：{error}"))?;
    let mut activated = portal
        .receive_activated()
        .await
        .map_err(|error| format!("监听快捷键按下事件失败：{error}"))?;
    let mut deactivated = portal
        .receive_deactivated()
        .await
        .map_err(|error| format!("监听快捷键释放事件失败：{error}"))?;
    let session = portal
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(|error| format!("创建桌面快捷键会话失败：{error}"))?;
    let shortcuts =
        [NewShortcut::new(ACTION_ID, "开始或停止语音输入").preferred_trigger(trigger.as_str())];
    let request = portal
        .bind_shortcuts(&session, &shortcuts, None, BindShortcutsOptions::default())
        .await
        .map_err(|error| format!("请求绑定快捷键失败：{error}"))?;
    let response = request.response().map_err(|error| match error {
        ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => {
            "用户取消了 Wayland 全局快捷键授权".to_string()
        }
        _ => format!("Wayland 全局快捷键绑定失败：{error}"),
    })?;
    if !response
        .shortcuts()
        .iter()
        .any(|shortcut| shortcut.id() == ACTION_ID)
    {
        let _ = session.close().await;
        return Err("桌面没有为 JustTalk 绑定全局快捷键".into());
    }

    let (cancel, cancelled) = tokio::sync::oneshot::channel();
    tauri::async_runtime::spawn(async move {
        let mut cancelled = std::pin::pin!(cancelled);
        loop {
            tokio::select! {
                _ = &mut cancelled => break,
                Some(event) = activated.next() => {
                    if event.shortcut_id() == ACTION_ID {
                        crate::dispatch_hotkey_event(&app, ShortcutState::Pressed);
                    }
                }
                Some(event) = deactivated.next() => {
                    if event.shortcut_id() == ACTION_ID {
                        crate::dispatch_hotkey_event(&app, ShortcutState::Released);
                    }
                }
                else => break,
            }
        }
        let _ = session.close().await;
    });

    Ok(PortalRegistration {
        hotkey: hotkey.to_owned(),
        cancel,
    })
}

#[cfg(any(target_os = "linux", test))]
pub fn tauri_hotkey_to_xdg(hotkey: &str) -> Result<String, String> {
    let parts: Vec<_> = hotkey.split('+').map(str::trim).collect();
    let (key, modifiers) = parts
        .split_last()
        .ok_or_else(|| "快捷键不能为空".to_string())?;
    if key.is_empty() || modifiers.iter().any(|part| part.is_empty()) {
        return Err("快捷键格式无效".into());
    }

    let mut converted = Vec::new();
    for modifier in modifiers {
        let value = match modifier.to_ascii_lowercase().as_str() {
            "commandorcontrol" | "control" | "ctrl" => "CTRL",
            "command" | "super" | "meta" => "LOGO",
            "shift" => "SHIFT",
            "option" | "alt" => "ALT",
            _ => return Err(format!("不支持的快捷键修饰键：{modifier}")),
        };
        if !converted.contains(&value) {
            converted.push(value);
        }
    }

    let normalized_key = match key.to_ascii_lowercase().as_str() {
        "space" => "space".to_string(),
        "enter" | "return" => "Return".to_string(),
        "escape" | "esc" => "Escape".to_string(),
        value
            if value.len() == 1
                && value
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric()) =>
        {
            value.to_string()
        }
        value
            if value.strip_prefix('f').is_some_and(|number| {
                number
                    .parse::<u8>()
                    .is_ok_and(|number| (1..=12).contains(&number))
            }) =>
        {
            value.to_ascii_uppercase()
        }
        _ => return Err(format!("Wayland 不支持该快捷键：{key}")),
    };
    if converted.is_empty() && !normalized_key.starts_with('F') {
        return Err("快捷键必须包含修饰键，F1-F12 可单独使用".into());
    }
    converted.push(normalized_key.as_str());
    Ok(converted.join("+"))
}

#[cfg(test)]
mod tests {
    use super::{LinuxBackend, linux_backend, tauri_hotkey_to_xdg};

    #[test]
    fn chooses_wayland_only_for_a_complete_wayland_session() {
        assert_eq!(
            linux_backend(Some("wayland"), Some("wayland-0")),
            LinuxBackend::WaylandPortal
        );
        assert_eq!(linux_backend(Some("x11"), None), LinuxBackend::X11);
        assert_eq!(linux_backend(Some("wayland"), None), LinuxBackend::X11);
    }

    #[test]
    fn converts_tauri_shortcuts_to_xdg_triggers() {
        assert_eq!(
            tauri_hotkey_to_xdg("CommandOrControl+Shift+Space").unwrap(),
            "CTRL+SHIFT+space"
        );
        assert_eq!(tauri_hotkey_to_xdg("Control+Alt+A").unwrap(), "CTRL+ALT+a");
        assert_eq!(tauri_hotkey_to_xdg("F8").unwrap(), "F8");
    }

    #[test]
    fn rejects_invalid_xdg_triggers() {
        assert!(tauri_hotkey_to_xdg("A").is_err());
        assert!(tauri_hotkey_to_xdg("Control+PageDown").is_err());
        assert!(tauri_hotkey_to_xdg("").is_err());
    }
}
