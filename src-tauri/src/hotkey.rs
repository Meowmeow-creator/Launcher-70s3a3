use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicI64, Ordering};

use tauri::{AppHandle, Emitter};

const MOD_NOREPEAT: u32 = 0x4000;
const WM_HOTKEY: u32 = 0x0312;
const PM_REMOVE: u32 = 0x0001;
const QS_ALLINPUT: u32 = 0x04FF;
const HOTKEY_ID: i32 = 0x70_53;
/// Packed (modifiers << 32) | vk, or -1 for "no hotkey". The pump thread polls it so a settings
/// change takes effect without restarting the launcher.
static DESIRED: AtomicI64 = AtomicI64::new(-1);

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Msg {
    hwnd: *mut c_void,
    message: u32,
    _pad: u32,
    wparam: usize,
    lparam: isize,
    point_x: i32,
    point_y: i32,
    private: u32,
    _tail: u32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterHotKey(window: *mut c_void, id: c_int, modifiers: u32, virtual_key: u32) -> i32;
    fn UnregisterHotKey(window: *mut c_void, id: c_int) -> i32;
    fn PeekMessageW(message: *mut Msg, window: *mut c_void, first: u32, last: u32, remove: u32) -> i32;
    fn TranslateMessage(message: *const Msg) -> i32;
    fn DispatchMessageW(message: *const Msg) -> isize;
    fn MsgWaitForMultipleObjects(count: u32, handles: *const c_void, wait_all: i32, milliseconds: u32, events: u32) -> u32;
}

pub fn pack(modifiers: u32, virtual_key: u32) -> i64 {
    ((modifiers as u64) << 32 | virtual_key as u64) as i64
}

pub fn request(modifiers: u32, virtual_key: u32) {
    DESIRED.store(pack(modifiers, virtual_key), Ordering::SeqCst);
}

pub fn clear() {
    DESIRED.store(-1, Ordering::SeqCst);
}

/// One process-wide pump thread. A global hotkey is the only way to fire 一键清图 while the game
/// window owns the foreground, so the listener cannot live in the webview.
pub fn spawn(app: AppHandle) {
    std::thread::Builder::new()
        .name("70s3a3-hotkey".to_string())
        .spawn(move || {
            let mut registered = -2i64;
            loop {
                let desired = DESIRED.load(Ordering::SeqCst);
                if desired != registered {
                    unsafe {
                        UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID);
                    }
                    registered = desired;
                    if desired >= 0 {
                        let modifiers = ((desired as u64) >> 32) as u32 | MOD_NOREPEAT;
                        let virtual_key = (desired & 0xFFFF_FFFF) as u32;
                        let ok =
                            unsafe { RegisterHotKey(std::ptr::null_mut(), HOTKEY_ID, modifiers, virtual_key) };
                        let payload = serde_json::json!({
                            "registered": ok != 0,
                            "modifiers": modifiers,
                            "virtualKey": virtual_key,
                        });
                        let _ = app.emit("hotkey-state", payload);
                    } else {
                        let _ = app.emit("hotkey-state", serde_json::json!({ "registered": false, "cleared": true }));
                    }
                }
                unsafe {
                    MsgWaitForMultipleObjects(0, std::ptr::null(), 0, 250, QS_ALLINPUT);
                    let mut message = Msg::default();
                    while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                        if message.message == WM_HOTKEY {
                            fire(app.clone());
                        } else {
                            TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                }
            }
        })
        .ok();
}

fn fire(app: AppHandle) {
    std::thread::spawn(move || {
        let payload = match run_kill_all() {
            Ok(message) => serde_json::json!({ "ok": true, "message": message, "source": "hotkey" }),
            Err(error) => serde_json::json!({ "ok": false, "message": error, "source": "hotkey" }),
        };
        let _ = app.emit("gm-event", payload);
    });
}

fn run_kill_all() -> Result<String, String> {
    let config = crate::config::load()?;
    let pairs: Vec<(&str, String)> = Vec::new();
    let reply = crate::gm::request(&config, "dungeon-killall", &pairs)?;
    Ok(reply
        .get("@message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("一键清图已排队。")
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_keeps_modifiers_above_the_virtual_key() {
        assert_eq!(pack(0x0003, 0x4b), 0x0000_0003_0000_004bi64);
        assert_eq!(pack(0, 0), 0);
    }

    /// Proves the hand-written user32 declarations actually work on this machine: the same flags the
    /// settings page produces must be accepted by RegisterHotKey.
    #[test]
    fn windows_accepts_the_registered_flag_shape() {
        const CONTROL_ALT: u32 = 0x2 | 0x1;
        const F9: u32 = 0x78;
        let id = HOTKEY_ID + 7;
        let ok = unsafe { RegisterHotKey(std::ptr::null_mut(), id, CONTROL_ALT | MOD_NOREPEAT, F9) };
        let error = std::io::Error::last_os_error();
        if ok != 0 {
            unsafe {
                UnregisterHotKey(std::ptr::null_mut(), id);
            }
        }
        assert!(ok != 0, "RegisterHotKey failed: {error}");
    }
}
