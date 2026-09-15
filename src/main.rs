#![windows_subsystem = "windows"]

mod autostart;
mod config;
mod crash;
mod guard;
mod icons;
mod state;
mod ui_main;
mod ui_settings;
mod util;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex};
use std::time::SystemTime;

use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostMessageW, TranslateMessage, MSG, SW_SHOWNORMAL,
};

use state::{App, Snapshot};

fn main() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // 盡早解析執行檔目錄並安裝閃退攔截器：後續任何 panic／存取違規都會寫進 crash.log
    let exe = std::env::current_exe().unwrap_or_default();
    let exe_path = exe.to_string_lossy().to_string();
    let exe_dir = exe
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let crash_path = format!("{}\\crash.log", exe_dir);
    crash::install_panic_hook(crash_path.clone());

    let args: Vec<String> = std::env::args().skip(1).collect();
    let no_elevate = args
        .iter()
        .any(|a| a.eq_ignore_ascii_case("--no-elevate"));
    let open_settings = args
        .iter()
        .any(|a| a.eq_ignore_ascii_case("--settings"));
    let admin = unsafe { IsUserAnAdmin() != 0 };

    if !admin && !no_elevate {
        if try_elevate(&args) {
            return;
        }
    }

    unsafe {
        let m = CreateMutexW(std::ptr::null(), 1, util::to_wide("Local\\SystemOOMGuardSingleInstance").as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = m;
            // 已有實例在跑：廣播激活訊息把它帶到前景後結束自己
            // （第二實例不得碰 sentinel，那是正在運行實例所擁有）
            PostMessageW(0xFFFF as HWND, ui_main::activate_message_id(), 0, 0);
            return;
        }
    }

    // 已取得單例，才可安全處理 sentinel：先讀取上次遺留標記（代表上次未正常結束），
    // 立即覆寫為本次標記；正常結束時（含 WM_ENDSESSION／訊息迴圈返回）再移除。
    let marker_path = format!("{}\\run.pid", exe_dir);
    let prev_abnormal = crash::take_stale_marker(&marker_path);
    crash::write_run_marker(&marker_path);

    // 診斷用：--crashtest 會故意 panic，用來驗證 panic hook 會寫入 crash.log、
    // 且本次 sentinel 未被清除 → 下次啟動會回報「上次未正常結束」。
    if args.iter().any(|a| a.eq_ignore_ascii_case("--crashtest")) {
        crash::append_crash(
            &crash_path,
            &format!("[{}] [Diagnostic] --crashtest：即將故意 panic\r\n", util::now_str()),
        );
        panic!("--crashtest 故意觸發的 panic（驗證閃退攔截器）");
    }

    let config_path = format!("{}\\config.json", exe_dir);
    let log_path = format!("{}\\System-OOM-Guard.log", exe_dir);

    let cfg = config::Config::load(&config_path);
    let icons = icons::IconSet::new();

    let app = Arc::new(App {
        running: AtomicBool::new(false),
        generation: std::sync::atomic::AtomicU32::new(0),
        pressured: AtomicBool::new(false),
        config: Mutex::new(cfg),
        snapshot: Mutex::new(Snapshot::default()),
        log_queue: Mutex::new(VecDeque::new()),
        last_kill: Mutex::new(SystemTime::now()),
        recent_kills: Mutex::new(HashMap::new()),
        log_path,
        config_path,
        exe_path,
        marker_path: marker_path.clone(),
        admin,
        exiting: AtomicBool::new(false),
        icons,
        sleep_lock: Mutex::new(()),
        sleep_cv: Condvar::new(),
    });

    {
        let cfg = app.config.lock().unwrap();
        if cfg.auto_start && !autostart::is_enabled() {
            autostart::enable(&app.exe_path);
            util::append_log(&app, "[System] 已註冊開機自啟排程任務");
        }
    }

    unsafe {
        ui_main::register_main_class();
        let hwnd: HWND = ui_main::create_main_window(app.clone());
        if hwnd.is_null() {
            return;
        }
        ui_main::add_tray(hwnd, &app);

        // 上次若有遺留 sentinel（未正常結束）→ 記錄疑似閃退，並同步寫入 crash.log
        if let Some(prev) = &prev_abnormal {
            let msg = format!(
                "[Crash] 偵測到上次執行未正常結束（可能閃退／堆疊溢出／OOM／被外部終止）。前次標記：{prev}"
            );
            util::append_log(&app, &msg);
            crash::append_crash(&crash_path, &format!("[{}] {msg}\r\n", util::now_str()));
        }

        // 程式啟動後自動啟用護欄（可於設定中關閉）
        if app.config.lock().unwrap().auto_guard {
            ui_main::guard_start(&app);
        }

        if open_settings {
            ui_settings::show_settings(app.clone(), hwnd);
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // 訊息迴圈正常返回（走 WM_DESTROY→PostQuitMessage 的乾淨結束路徑）：移除 sentinel
    crash::remove_run_marker(&marker_path);
}

fn try_elevate(args: &[String]) -> bool {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return false,
    };
    let params: Vec<String> = args
        .iter()
        .filter(|a| !a.eq_ignore_ascii_case("--no-elevate"))
        .cloned()
        .collect();
    let joined = params.join(" ");
    unsafe {
        let r = ShellExecuteW(
            std::ptr::null_mut(),
            util::to_wide("runas").as_ptr(),
            util::to_wide(&exe.to_string_lossy()).as_ptr(),
            if joined.is_empty() {
                std::ptr::null()
            } else {
                util::to_wide(&joined).as_ptr()
            },
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
        (r as isize) > 32
    }
}