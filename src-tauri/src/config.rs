use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyMode {
    #[default]
    Free,
    Normal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub app_id: String,
    pub access_token: String,
    pub language: String,
    pub hotkey: String,
    pub hotkey_mode: HotkeyMode,
    pub auto_paste: bool,
    pub use_gzip: bool,
    pub enable_punc: bool,
    pub enable_ddc: bool,
    pub hotwords: String,
    pub debug_enabled: bool,
    pub pause_media_during_recording: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            access_token: String::new(),
            language: "zh-CN".into(),
            hotkey: "CommandOrControl+Shift+Space".into(),
            hotkey_mode: HotkeyMode::Free,
            auto_paste: true,
            use_gzip: false,
            enable_punc: true,
            enable_ddc: false,
            hotwords: String::new(),
            debug_enabled: false,
            pause_media_during_recording: true,
        }
    }
}

fn path() -> Result<PathBuf, String> {
    let base = dirs::config_dir().ok_or("无法确定系统配置目录")?;
    Ok(base.join("just-talk-slim").join("config.json"))
}

pub fn load() -> AppConfig {
    path()
        .ok()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(config: &AppConfig) -> Result<(), String> {
    let path = path()?;
    let parent = path.parent().ok_or("配置目录无效")?;
    fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败：{e}"))?;
    let text = serde_json::to_string_pretty(config).map_err(|e| format!("序列化配置失败：{e}"))?;
    fs::write(path, text).map_err(|e| format!("保存配置失败：{e}"))
}
