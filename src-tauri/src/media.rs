//! Best-effort control of media sessions while recording.
//!
//! This intentionally controls registered media sessions, not the system audio
//! mixer. A token only remembers sessions that were playing before JustTalk
//! paused them, so pre-existing paused media is never started by us.

#[derive(Debug)]
pub struct PauseToken {
    platform: platform::Token,
}

impl PauseToken {
    pub fn pause_current() -> Option<Self> {
        platform::pause_current().map(|platform| Self { platform })
    }

    pub fn resume(self) {
        platform::resume(self.platform);
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::time::Duration;

    use media_remote::{Controller, NowPlayingJXA};

    #[derive(Debug)]
    pub struct Token {
        bundle_id: Option<String>,
    }

    pub fn pause_current() -> Option<Token> {
        let controller = NowPlayingJXA::new(Duration::from_millis(50));
        let info = controller.get_info();
        let current = info.as_ref()?;
        if current.is_playing != Some(true) {
            return None;
        }
        let bundle_id = current.bundle_id.clone();
        drop(info);
        controller.pause().then_some(Token { bundle_id })
    }

    pub fn resume(token: Token) {
        let controller = NowPlayingJXA::new(Duration::from_millis(50));
        let info = controller.get_info();
        let Some(current) = info.as_ref() else { return };
        // Do not control a different player or a session the user already resumed.
        if current.bundle_id != token.bundle_id || current.is_playing != Some(false) {
            return;
        }
        drop(info);
        let _ = controller.play();
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use mpris::{PlaybackStatus, PlayerFinder};

    #[derive(Debug)]
    pub struct Token {
        bus_names: Vec<String>,
    }

    pub fn pause_current() -> Option<Token> {
        let finder = PlayerFinder::new().ok()?;
        let mut bus_names = Vec::new();
        for player in finder.find_all().ok()? {
            if player.get_playback_status() == Ok(PlaybackStatus::Playing) && player.pause().is_ok()
            {
                bus_names.push(player.bus_name().to_owned());
            }
        }
        (!bus_names.is_empty()).then_some(Token { bus_names })
    }

    pub fn resume(token: Token) {
        let Ok(finder) = PlayerFinder::new() else {
            return;
        };
        let Ok(players) = finder.find_all() else {
            return;
        };
        for player in players {
            if token.bus_names.iter().any(|name| name == player.bus_name())
                && player.get_playback_status() == Ok(PlaybackStatus::Paused)
            {
                let _ = player.play();
            }
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use futures::executor::block_on;
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSessionManager as SessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
    };

    #[derive(Debug)]
    pub struct Token {
        source_ids: Vec<String>,
    }

    pub fn pause_current() -> Option<Token> {
        let manager = block_on(SessionManager::RequestAsync().ok()?).ok()?;
        let sessions = manager.GetSessions().ok()?;
        let mut source_ids = Vec::new();
        for session in sessions {
            let status = session.GetPlaybackInfo().ok()?.PlaybackStatus().ok()?;
            if status == PlaybackStatus::Playing
                && block_on(session.TryPauseAsync().ok()?).ok()? == true
            {
                source_ids.push(session.SourceAppUserModelId().ok()?.to_string());
            }
        }
        (!source_ids.is_empty()).then_some(Token { source_ids })
    }

    pub fn resume(token: Token) {
        let Ok(operation) = SessionManager::RequestAsync() else {
            return;
        };
        let Ok(manager) = block_on(operation) else {
            return;
        };
        let Ok(sessions) = manager.GetSessions() else {
            return;
        };
        for session in sessions {
            let Ok(source_id) = session.SourceAppUserModelId() else {
                continue;
            };
            let Ok(info) = session.GetPlaybackInfo() else {
                continue;
            };
            if token.source_ids.iter().any(|id| id == source_id.as_str())
                && info.PlaybackStatus() == Ok(PlaybackStatus::Paused)
            {
                if let Ok(operation) = session.TryPlayAsync() {
                    let _ = block_on(operation);
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
mod platform {
    #[derive(Debug)]
    pub struct Token;
    pub fn pause_current() -> Option<Token> {
        None
    }
    pub fn resume(_: Token) {}
}
