use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct PermissionStatus {
    pub required: bool,
    pub microphone: bool,
    pub accessibility: bool,
    pub all_granted: bool,
}

#[cfg(not(target_os = "macos"))]
pub fn status() -> PermissionStatus {
    PermissionStatus {
        required: false,
        microphone: true,
        accessibility: true,
        all_granted: true,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn request(_kind: &str) -> Result<PermissionStatus, String> {
    Ok(status())
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{
        ffi::{CString, c_void},
        process::Command,
    };

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

    use super::PermissionStatus;

    type CFTypeRef = *const c_void;
    type CFAllocatorRef = *const c_void;
    type CFDictionaryRef = *const c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
        static kAXTrustedCheckOptionPrompt: CFTypeRef;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFBooleanTrue: CFTypeRef;
        fn CFDictionaryCreate(
            allocator: CFAllocatorRef,
            keys: *const CFTypeRef,
            values: *const CFTypeRef,
            count: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> CFDictionaryRef;
        fn CFRelease(value: CFTypeRef);
    }

    fn microphone_granted() -> bool {
        let Some(media_type) = (unsafe { AVMediaTypeAudio }) else {
            return false;
        };
        unsafe {
            AVCaptureDevice::authorizationStatusForMediaType(media_type)
                == AVAuthorizationStatus::Authorized
        }
    }

    pub fn status() -> PermissionStatus {
        let microphone = microphone_granted();
        let accessibility = unsafe { AXIsProcessTrusted() };
        PermissionStatus {
            required: true,
            microphone,
            accessibility,
            all_granted: microphone && accessibility,
        }
    }

    pub fn request(kind: &str) -> Result<PermissionStatus, String> {
        match kind {
            "microphone" => {
                let Some(media_type) = (unsafe { AVMediaTypeAudio }) else {
                    return Err("系统未提供麦克风媒体类型".into());
                };
                let handler = RcBlock::new(|_granted: Bool| {});
                unsafe {
                    AVCaptureDevice::requestAccessForMediaType_completionHandler(
                        media_type, &handler,
                    );
                }
            }
            "accessibility" => unsafe {
                let keys = [kAXTrustedCheckOptionPrompt];
                let values = [kCFBooleanTrue];
                let options = CFDictionaryCreate(
                    std::ptr::null(),
                    keys.as_ptr(),
                    values.as_ptr(),
                    1,
                    std::ptr::null(),
                    std::ptr::null(),
                );
                AXIsProcessTrustedWithOptions(options);
                if !options.is_null() {
                    CFRelease(options);
                }
            },
            _ => return Err("未知的权限类型".into()),
        }
        Ok(status())
    }

    pub fn open_settings(kind: &str) -> Result<(), String> {
        let pane = match kind {
            "microphone" => "Privacy_Microphone",
            "accessibility" => "Privacy_Accessibility",
            _ => return Err("未知的权限类型".into()),
        };
        let url = CString::new(format!(
            "x-apple.systempreferences:com.apple.preference.security?{pane}"
        ))
        .unwrap();
        Command::new("open")
            .arg(url.to_str().unwrap())
            .spawn()
            .map_err(|e| format!("无法打开系统设置：{e}"))?;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub use macos::{request, status};

#[cfg(target_os = "macos")]
pub fn open_settings(kind: &str) -> Result<(), String> {
    macos::open_settings(kind)
}

#[cfg(not(target_os = "macos"))]
pub fn open_settings(_kind: &str) -> Result<(), String> {
    Ok(())
}
