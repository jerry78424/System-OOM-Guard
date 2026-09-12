use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, MutexGuard};

use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::state::App;

/// 常駐 log 控制代碼：省去每行 CreateFile/CloseHandle；
/// append 模式（FILE_APPEND_DATA）下單次 write_all 的跨進程原子附加語意不變
static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

fn lock_ok<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ptr, len) })
}

pub fn now_str() -> String {
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    )
}

pub fn fmt_thousands(v: u64) -> String {
    let s = v.to_string();
    let bytes = s.as_bytes();
    let n = bytes.len();
    let mut out = String::with_capacity(n + n / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (n - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Append a log line to the file and to the in-memory queue for the UI.
pub fn append_log(app: &App, message: &str) {
    let line = format!("[{}] {}\r\n", now_str(), message);
    {
        let mut slot = lock_ok(&LOG_FILE);
        let ok = match slot.as_mut() {
            Some(f) => f.write_all(line.as_bytes()).is_ok(),
            None => false,
        };
        if !ok {
            // 無控制代碼或寫入失敗：重開後補寫同一行（仍是單次原子附加）
            match OpenOptions::new().create(true).append(true).open(&app.log_path) {
                Ok(mut f) => {
                    if let Err(e) = f.write_all(line.as_bytes()) {
                        eprintln!("log write failed: {e}");
                    }
                    *slot = Some(f);
                }
                Err(e) => {
                    eprintln!("log open failed: {e}");
                    drop(slot);
                    lock_ok(&app.log_queue).push_back(format!("[LogErr] 無法寫入 log：{e}"));
                }
            }
        }
    }
    lock_ok(&app.log_queue).push_back(line.trim_end().to_string());
}

pub fn copy_wide_into(dst: &mut [u16], s: &str) {
    let mut v = s.encode_utf16();
    for slot in dst.iter_mut() {
        *slot = v.next().unwrap_or(0);
    }
}

pub fn open_with_shell(target: &str, params: Option<&str>) {
    let t = to_wide(target);
    let verb = to_wide("open");
    let p = params.map(|s| to_wide(s));
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            t.as_ptr(),
            p.as_ref().map(|v| v.as_ptr()).unwrap_or(std::ptr::null()),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}