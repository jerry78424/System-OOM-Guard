//! 閃退偵測與記錄。
//!
//! `panic = "abort"` 下，多數 panic 與 Windows 存取違規會在 abort 前經過
//! panic hook → 寫入 `crash.log`（thread／訊息／檔案:行號）。
//! 但堆疊溢出、OOM `handle_alloc_error`、被外部 `TerminateProcess` 等
//! **不會**經過 hook 的死法，改由下次啟動時的 sentinel（`run.pid`）偵測：
//! 正常結束會移除標記，殘留即代表上次非正常結束。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic;

use crate::util;

/// 安裝 panic 攔截器：把 panic 的 thread、訊息、來源位置附加到 crash.log。
/// 需在程式盡早（任何可能 panic 的碼之前）呼叫。
pub fn install_panic_hook(crash_path: String) {
    panic::set_hook(Box::new(move |info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "(非字串 panic payload)".to_string()
        };
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<未命名>");
        let loc = match info.location() {
            Some(l) => format!("{}:{}:{}", l.file(), l.line(), l.column()),
            None => "<未知位置>".to_string(),
        };
        let line = format!(
            "[{}] PANIC thread='{thread_name}' 來源 {loc}：{payload}\r\n",
            util::now_str()
        );
        write_crash(&crash_path, &line);
    }));
}

/// 附加一行到 crash.log（盡力而為；hook 內不可再 panic，故忽略所有錯誤）。
pub fn append_crash(crash_path: &str, text: &str) {
    write_crash(crash_path, text);
}

fn write_crash(crash_path: &str, text: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(crash_path) {
        let _ = f.write_all(text.as_bytes());
        let _ = f.flush();
    }
}

/// 寫入執行中標記（目前 PID + 啟動時間）。正常結束時由 `remove_run_marker` 移除。
pub fn write_run_marker(marker_path: &str) {
    let content = format!("pid={}\nstart={}\n", std::process::id(), util::now_str());
    let _ = fs::write(marker_path, content);
}

/// 讀取上次遺留的標記（有代表上次未正常結束）。呼叫端讀完後應立即覆寫為本次標記。
pub fn take_stale_marker(marker_path: &str) -> Option<String> {
    fs::read_to_string(marker_path)
        .ok()
        .map(|s| s.replace("\r\n", " ").replace('\n', " ").trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 正常（或系統可預期結束：關機／登出）離開時移除標記。
pub fn remove_run_marker(marker_path: &str) {
    let _ = fs::remove_file(marker_path);
}
