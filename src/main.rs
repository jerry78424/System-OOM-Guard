#![windows_subsystem = "windows"]

mod autostart;
mod config;
mod guard;
mod icons;
mod state;
mod ui_main;
mod ui_settings;
mod util;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Instant, SystemTime};

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
            PostMessageW(0xFFFF as HWND, ui_main::activate_message_id(), 0, 0);
            return;
        }
    }

    let exe = std::env::current_exe().unwrap_or_default();
    let exe_path = exe.to_string_lossy().to_string();
    let exe_dir = exe
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
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
        // 讓首次進入壓力時能立即回收 standby：把基準設為「60 秒前」。
        // 但開機 60 秒內 Instant::now()-60s 會下溢 panic（配合 panic=abort 即閃退，
        // 恰好命中開機自啟情境），故用 checked_sub 回退為 now，絕不下溢。
        last_standby: Mutex::new(
            Instant::now()
                .checked_sub(std::time::Duration::from_secs(60))
                .unwrap_or_else(Instant::now),
        ),
        log_path,
        config_path,
        exe_path,
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