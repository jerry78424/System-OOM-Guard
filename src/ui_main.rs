use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, GetSysColorBrush, InvalidateRect, SetBkMode,
    SetTextColor, CLEARTYPE_QUALITY, COLOR_WINDOW, DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE,
    FW_BOLD, FW_NORMAL, HBRUSH, HFONT, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{EM_REPLACESEL, EM_SCROLLCARET, EM_SETSEL};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, AppendMenuW, ChangeWindowMessageFilterEx, CreatePopupMenu,
    CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetClientRect, GetCursorPos, GetSystemMetrics,
    GetWindowLongPtrW, GetWindowRect, KillTimer, LoadCursorW, LoadImageW, MINMAXINFO, MoveWindow,
    PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SendMessageW,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow,
    TrackPopupMenu, CW_USEDEFAULT, WNDCLASSEXW, CREATESTRUCTW, IMAGE_ICON, LR_DEFAULTSIZE,
    MSGFLT_ALLOW, SW_RESTORE, WM_APP, WM_GETMINMAXINFO, WM_POWERBROADCAST,
    BS_PUSHBUTTON, CS_HREDRAW, CS_VREDRAW, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY,
    GWLP_USERDATA, HICON, HMENU, IDC_ARROW, MF_SEPARATOR, MF_STRING, SM_CXSCREEN, SM_CYSCREEN,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE,
    SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_CLOSE, WM_COMMAND,
    WM_CREATE, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_EXITSIZEMOVE, WM_GETTEXTLENGTH,
    WM_LBUTTONDBLCLK, WM_RBUTTONUP, WM_SETFONT, WM_SIZE, WM_TIMER, WM_USER, WS_CHILD,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL, WS_EX_CLIENTEDGE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::PBT_APMRESUMEAUTOMATIC;

use crate::state::App;
use crate::ui_settings;
use crate::util;

pub const MAIN_CLASS: &str = "OOMGuardMainWnd";
const WM_TRAY: u32 = WM_USER + 1;

static ACTIVATE_MSG: AtomicU32 = AtomicU32::new(0);
static TASKBAR_MSG: AtomicU32 = AtomicU32::new(0);

/// explorer 重新啟動時廣播 TaskbarCreated；收到要重建托盤圖示
fn taskbar_message_id() -> u32 {
    let v = TASKBAR_MSG.load(Ordering::SeqCst);
    if v != 0 {
        return v;
    }
    let id = unsafe { RegisterWindowMessageW(util::to_wide("TaskbarCreated").as_ptr()) };
    TASKBAR_MSG.store(id, Ordering::SeqCst);
    TASKBAR_MSG.load(Ordering::SeqCst)
}

/// 註冊一次性的啟動視窗訊息；第二個實例用 HWND_BROADCAST 廣播它，
/// 已運行實例收到後把主視窗帶到前景（跨提權等級也可送達）
pub fn activate_message_id() -> u32 {
    let v = ACTIVATE_MSG.load(Ordering::SeqCst);
    if v != 0 {
        return v;
    }
    let id = unsafe { RegisterWindowMessageW(util::to_wide("OOMGuardActivateMsg").as_ptr()) };
    ACTIVATE_MSG.store(id.max(WM_APP + 1), Ordering::SeqCst);
    ACTIVATE_MSG.load(Ordering::SeqCst)
}

const IDC_STATUS: usize = 6;
const IDC_ADMIN: usize = 7;
const IDC_LOG: usize = 8;
const IDT_TIMER: usize = 1;
const TRAY_ID: u32 = 100;
const MENU_OPEN: usize = 1001;
const MENU_SETTINGS: usize = 1002;
const MENU_EXIT: usize = 1003;

const COLOR_GRAY: u32 = 0x00808080;
const COLOR_RED: u32 = 0x000000FF;
const COLOR_GREEN: u32 = 0x00008000;
const COLOR_ORANGE: u32 = 0x00008CFF;

struct MainCtx {
    app: Arc<App>,
    hwnd: HWND,
    status: HWND,
    admin: HWND,
    log: HWND,
    start: HWND,
    stop: HWND,
    settings: HWND,
    openlog: HWND,
    openfolder: HWND,
    font_status: HFONT,
    font_log: HFONT,
    font_btn: HFONT,
    status_color: AtomicU32,
    // 變更偵測快取：內容沒變就不呼叫 Win32（避免每秒重繪與托盤 IPC）
    ui_cache: std::sync::Mutex<UiCache>,
}

#[derive(Default)]
struct UiCache {
    status_text: String,
    tray_tip: String,
    tray_icon_kind: u8,
    running_shown: bool,
}

pub unsafe fn register_main_class() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let mut wc: WNDCLASSEXW = std::mem::zeroed();
        wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        wc.style = CS_HREDRAW | CS_VREDRAW;
        wc.lpfnWndProc = Some(main_wndproc);
        wc.hInstance = GetModuleHandleW(std::ptr::null());
        wc.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
        wc.hIcon = LoadImageW(
            GetModuleHandleW(std::ptr::null()),
            1 as *const u16,
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE,
        ) as HICON;
        wc.hbrBackground = (COLOR_WINDOW as isize + 1) as HBRUSH;
        let cls = util::to_wide(MAIN_CLASS);
        wc.lpszClassName = cls.as_ptr();
        RegisterClassExW(&wc);
    });
}

pub unsafe fn create_main_window(app: Arc<App>) -> HWND {
    let geom = {
        let cfg = app.config.lock().unwrap();
        (
            cfg.window_x,
            cfg.window_y,
            cfg.window_width,
            cfg.window_height,
        )
    };
    let ctx = Box::new(MainCtx {
        app,
        hwnd: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        admin: std::ptr::null_mut(),
        log: std::ptr::null_mut(),
        start: std::ptr::null_mut(),
        stop: std::ptr::null_mut(),
        settings: std::ptr::null_mut(),
        openlog: std::ptr::null_mut(),
        openfolder: std::ptr::null_mut(),
        font_status: std::ptr::null_mut(),
        font_log: std::ptr::null_mut(),
        font_btn: std::ptr::null_mut(),
        status_color: AtomicU32::new(COLOR_GREEN),
        ui_cache: std::sync::Mutex::new(UiCache::default()),
    });
    let ctx_ptr = Box::into_raw(ctx);
    let hwnd = CreateWindowExW(
        0,
        util::to_wide(MAIN_CLASS).as_ptr(),
        util::to_wide("System-OOM-Guard").as_ptr(),
        WS_OVERLAPPEDWINDOW as u32,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        760,
        520,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        ctx_ptr as *const c_void,
    );
    if hwnd.is_null() {
        drop(Box::from_raw(ctx_ptr));
        return std::ptr::null_mut();
    }
    (*ctx_ptr).hwnd = hwnd;
    match geom {
        (Some(x), Some(y), Some(w), Some(h))
            if w >= 320 && h >= 240 && rect_intersects_screen(x, y, w, h) =>
        {
            SetWindowPos(hwnd, std::ptr::null_mut(), x, y, w, h, SWP_NOZORDER);
        }
        _ => center_on_primary(hwnd, 760, 520),
    }
    ShowWindow(hwnd, SW_SHOW);
    hwnd
}

unsafe fn rect_intersects_screen(x: i32, y: i32, w: i32, h: i32) -> bool {
    let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
    let ow = (x + w).min(vx + vw) - x.max(vx);
    let oh = (y + h).min(vy + vh) - y.max(vy);
    ow > 0 && oh > 0
}

unsafe fn save_geometry(c: &MainCtx) {
    let mut rc: RECT = std::mem::zeroed();
    GetWindowRect(c.hwnd, &mut rc);
    let mut cfg = c.app.config.lock().unwrap();
    cfg.window_x = Some(rc.left);
    cfg.window_y = Some(rc.top);
    cfg.window_width = Some(rc.right - rc.left);
    cfg.window_height = Some(rc.bottom - rc.top);
    cfg.save(&c.app.config_path);
}

unsafe fn center_on_primary(hwnd: HWND, w: i32, h: i32) {
    let sw = GetSystemMetrics(SM_CXSCREEN);
    let sh = GetSystemMetrics(SM_CYSCREEN);
    let x = ((sw - w) / 2).max(0);
    let y = ((sh - h) / 2).max(0);
    SetWindowPos(hwnd, std::ptr::null_mut(), x, y, w, h, SWP_NOZORDER);
}

unsafe fn create_ctl(
    parent: HWND,
    class: &[u16],
    text: &[u16],
    style: u32,
    ex: u32,
    id: usize,
) -> HWND {
    CreateWindowExW(
        ex,
        class.as_ptr(),
        text.as_ptr(),
        style,
        0,
        0,
        0,
        0,
        parent,
        id as HMENU,
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    )
}

unsafe fn create_font(px: i32, bold: bool, face: &[u16]) -> HFONT {
    let weight = if bold { FW_BOLD } else { FW_NORMAL };
    CreateFontW(
        -px,
        0,
        0,
        0,
        weight as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        0,
        0,
        CLEARTYPE_QUALITY as u32,
        (DEFAULT_PITCH | FF_DONTCARE) as u32,
        face.as_ptr(),
    )
}

unsafe fn recreate_fonts(c: &mut MainCtx, dpi: u32) {
    let scale = dpi as f32 / 96.0;
    let status_px = (15.0 * scale).round() as i32;
    let log_px = (12.0 * scale).round() as i32;
    let btn_px = (12.0 * scale).round() as i32;
    let status_font = create_font(status_px, true, &util::to_wide("Segoe UI"));
    let log_font = create_font(log_px, false, &util::to_wide("Consolas"));
    let btn_font = create_font(btn_px, false, &util::to_wide("Segoe UI"));
    if !c.font_status.is_null() {
        DeleteObject(c.font_status);
    }
    if !c.font_log.is_null() {
        DeleteObject(c.font_log);
    }
    if !c.font_btn.is_null() {
        DeleteObject(c.font_btn);
    }
    c.font_status = status_font;
    c.font_log = log_font;
    c.font_btn = btn_font;
    SendMessageW(c.status, WM_SETFONT, c.font_status as WPARAM, 1);
    SendMessageW(c.admin, WM_SETFONT, c.font_status as WPARAM, 1);
    SendMessageW(c.log, WM_SETFONT, c.font_log as WPARAM, 1);
    let btns = [c.start, c.stop, c.settings, c.openlog, c.openfolder];
    for h in btns {
        SendMessageW(h, WM_SETFONT, c.font_btn as WPARAM, 1);
    }
}

unsafe fn layout_main(c: &mut MainCtx, dpi: u32) {
    let hwnd = c.hwnd;
    let scale = dpi as f32 / 96.0;
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rc);
    let cw = rc.right - rc.left;
    let ch = rc.bottom - rc.top;
    let s = |v: i32| (v as f32 * scale).round() as i32;
    let status_h = s(36);
    let admin_h = s(24);
    let btn_h = s(46);
    let btn_y = ch - btn_h;
    MoveWindow(c.status, 0, 0, cw, status_h, 1);
    MoveWindow(c.admin, 0, status_h, cw, admin_h, 1);
    MoveWindow(c.log, 0, status_h + admin_h, cw, btn_y - status_h - admin_h, 1);
    let mut bx = s(10);
    let by = btn_y + s(8);
    let bhh = s(30);
    for (h, wd) in [
        (c.start, 90),
        (c.stop, 90),
        (c.settings, 80),
        (c.openlog, 110),
        (c.openfolder, 110),
    ] {
        MoveWindow(h, bx, by, s(wd), bhh, 1);
        bx += s(wd) + s(8);
    }
}

unsafe fn load_log_history(c: &mut MainCtx) {
    use std::io::{Read, Seek, SeekFrom};
    // 只讀檔尾（最後 512KB）：log 檔再大，啟動尖峰記憶體也有界。
    // 若切割點落在某行中間，該殘行開頭不是 '['，下方過濾會自然丟棄。
    const TAIL_BYTES: u64 = 512 * 1024;
    let mut file = match std::fs::File::open(&c.app.log_path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let len = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => return,
    };
    if file.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES))).is_err() {
        return;
    }
    let mut bytes = Vec::with_capacity(len.min(TAIL_BYTES) as usize);
    if file.read_to_end(&mut bytes).is_err() {
        return;
    }
    let text = String::from_utf8_lossy(&bytes);
    // 只取完整行、重新以 \r\n 組合，避免歷史檔內的殘缺片段破壞顯示
    let lines: Vec<&str> = text.lines().filter(|l| l.starts_with('[')).collect();
    let from = lines.len().saturating_sub(500);
    let tail = lines[from..].join("\r\n");
    SetWindowTextW(c.log, util::to_wide(&tail).as_ptr());
}

unsafe fn append_log_block(hwnd: HWND, text: &str) {
    let mut len = SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0);
    if len > 400_000 {
        // 平滑裁切：刪掉最舊的一半而非整段清空。
        // EDIT 內部緩衝倍增成長，整段清空會瞬間失去全部歷史且緩衝未必歸還；
        // 對半刪可把常駐上限壓在 ~600K 字元內並保留近期記錄
        SendMessageW(hwnd, EM_SETSEL, 0, (len / 2) as isize);
        SendMessageW(hwnd, EM_REPLACESEL, 0, util::to_wide("").as_ptr() as isize);
        len = SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0);
    }
    // 用實際長度定位到結尾（比 -1,-1 寫法在各 EDIT 實作上更可靠）
    SendMessageW(hwnd, EM_SETSEL, len as usize, len as isize);
    let mut w = util::to_wide(text);
    w.insert(0, b'\n' as u16);
    w.insert(0, b'\r' as u16);
    SendMessageW(hwnd, EM_REPLACESEL, 0, w.as_ptr() as isize);
    SendMessageW(hwnd, EM_SCROLLCARET, 0, 0);
}

unsafe fn update_status(c: &mut MainCtx) {
    let app = &c.app;
    // trigger 隨 run_cycle 快取在 snapshot，UI 每秒 tick 不必再鎖 config
    let (running, pressured, used, avail, total, last_action, trigger) = {
        let snap = app.snapshot.lock().unwrap();
        (
            snap.running,
            snap.pressured,
            snap.used_percent,
            snap.avail_mb,
            snap.total_mb,
            snap.last_action.clone(),
            snap.trigger_percent as f64,
        )
    };
    let state = if !running {
        "已停止"
    } else if pressured {
        "壓力介入中"
    } else {
        "運行中"
    };
    let text = format!(
        "[{}] 使用率 {:.1}%  |  可用 {} MB / {} MB  |  上次動作：{}",
        state,
        used,
        util::fmt_thousands(avail),
        util::fmt_thousands(total),
        last_action
    );
    // 壓力狀態（>= 觸發後直到 < 恢復門檻）一律以紅色呈現
    let color = if !running {
        COLOR_GRAY
    } else if pressured || used >= trigger {
        COLOR_RED
    } else {
        COLOR_GREEN
    };

    // ---- 變更偵測：內容沒變就完全不碰 Win32 ----
    let mut cache = c.ui_cache.lock().unwrap();

    if text != cache.status_text {
        SetWindowTextW(c.status, util::to_wide(&text).as_ptr());
        c.status_color.store(color, Ordering::SeqCst);
        InvalidateRect(c.status, std::ptr::null(), 1);
        cache.status_text = text;
    } else if c.status_color.load(Ordering::SeqCst) != color {
        c.status_color.store(color, Ordering::SeqCst);
        InvalidateRect(c.status, std::ptr::null(), 1);
    }

    let icon_kind: u8 = if !running {
        0
    } else if pressured || used >= trigger {
        1
    } else {
        2
    };
    let icon = match icon_kind {
        0 => app.icons.gray,
        1 => app.icons.red,
        _ => app.icons.green,
    };
    let tip = format!("System-OOM-Guard - 使用率 {:.1}%", used);
    if tip != cache.tray_tip || icon_kind != cache.tray_icon_kind {
        tray_set(c.hwnd, &tip, icon);
        cache.tray_tip = tip;
        cache.tray_icon_kind = icon_kind;
    }

    if running != cache.running_shown {
        EnableWindow(c.start, (!running) as i32);
        EnableWindow(c.stop, running as i32);
        cache.running_shown = running;
    }

    // 批次附加：整批新 log 一次 SendMessage 完成，
    // 爆發期（一次殺多個進程）從 N 次重繪＋捲動降為 1 次
    let batch = {
        let mut q = app.log_queue.lock().unwrap();
        let mut out = String::new();
        while let Some(line) = q.pop_front() {
            out.push_str(&line);
            out.push_str("\r\n");
        }
        out
    };
    if !batch.is_empty() {
        append_log_block(c.log, &batch);
    }
}

pub unsafe fn add_tray(hwnd: HWND, app: &App) {
    let mut nid: windows_sys::Win32::UI::Shell::NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<windows_sys::Win32::UI::Shell::NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = app.icons.green;
    util::copy_wide_into(&mut nid.szTip, "System-OOM-Guard");
    Shell_NotifyIconW(NIM_ADD, &nid);
}

unsafe fn remove_tray(hwnd: HWND) {
    let mut nid: windows_sys::Win32::UI::Shell::NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<windows_sys::Win32::UI::Shell::NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    Shell_NotifyIconW(NIM_DELETE, &nid);
}

unsafe fn tray_set(hwnd: HWND, tip: &str, icon: HICON) {
    let mut nid: windows_sys::Win32::UI::Shell::NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<windows_sys::Win32::UI::Shell::NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    nid.uFlags = NIF_TIP | NIF_ICON;
    nid.hIcon = icon;
    util::copy_wide_into(&mut nid.szTip, tip);
    Shell_NotifyIconW(NIM_MODIFY, &nid);
}

unsafe fn tray_balloon(hwnd: HWND, title: &str, info: &str) {
    let mut nid: windows_sys::Win32::UI::Shell::NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<windows_sys::Win32::UI::Shell::NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    nid.uFlags = NIF_INFO;
    util::copy_wide_into(&mut nid.szInfo, info);
    util::copy_wide_into(&mut nid.szInfoTitle, title);
    nid.dwInfoFlags = NIIF_INFO;
    Shell_NotifyIconW(NIM_MODIFY, &nid);
}

pub fn guard_start(app: &Arc<App>) {
    if app.running.swap(true, Ordering::SeqCst) {
        return;
    }
    *app.last_kill.lock().unwrap() = SystemTime::now();
    // 重置滯回狀態：重新啟動視為全新監測週期
    app.pressured.store(false, Ordering::SeqCst);
    {
        let mut snap = app.snapshot.lock().unwrap();
        snap.running = true;
        snap.pressured = false;
        // 立即填入門檻，避免第一個週期前 UI 誤判顏色
        snap.trigger_percent = app.config.lock().unwrap().trigger_used_percent;
    }
    crate::util::append_log(app, "記憶體護欄已啟動");
    let gen = app.generation.fetch_add(1, Ordering::SeqCst) + 1;
    // 喚醒舊世代執行緒：讓它立即看到 generation 已變更並退出
    crate::guard::wake_guard(app);
    crate::guard::spawn_guard(app.clone(), gen);
}

fn guard_stop(app: &Arc<App>) {
    app.running.store(false, Ordering::SeqCst);
    // 立即喚醒睡眠中的護欄執行緒（原本最多要等 250ms 切片醒來）
    crate::guard::wake_guard(app);
    app.pressured.store(false, Ordering::SeqCst);
    let mut snap = app.snapshot.lock().unwrap();
    snap.running = false;
    snap.pressured = false;
    drop(snap);
    crate::util::append_log(app, "記憶體護欄已停止");
}

unsafe fn exit_app(c: &mut MainCtx) {
    c.app.exiting.store(true, Ordering::SeqCst);
    ShowWindow(c.hwnd, SW_HIDE);
    DestroyWindow(c.hwnd);
}

unsafe extern "system" fn main_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let act = ACTIVATE_MSG.load(Ordering::SeqCst);
    if act != 0 && msg == act {
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
        return 0;
    }
    let tb = TASKBAR_MSG.load(Ordering::SeqCst);
    if tb != 0 && msg == tb {
        // explorer 重啟：重建托盤圖示
        let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MainCtx;
        if !ctx.is_null() {
            add_tray(hwnd, &(*ctx).app);
        }
        return 0;
    }
    match msg {
        WM_CREATE => {
            let cs = lparam as *const CREATESTRUCTW;
            let ctx = (*cs).lpCreateParams as *mut MainCtx;
            if ctx.is_null() {
                return -1 as LRESULT;
            }
            (*ctx).hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, ctx as isize);
            let c = &mut *ctx;
            c.status = create_ctl(
                hwnd,
                &util::to_wide("STATIC"),
                &util::to_wide(""),
                WS_CHILD | WS_VISIBLE,
                0,
                IDC_STATUS,
            );
            let admin_text = if c.app.admin {
                ""
            } else {
                "警告：未以系統管理員身分執行，終止進程與清空快取功能可能失敗。"
            };
            c.admin = create_ctl(
                hwnd,
                &util::to_wide("STATIC"),
                &util::to_wide(admin_text),
                WS_CHILD | WS_VISIBLE,
                0,
                IDC_ADMIN,
            );
            c.log = create_ctl(
                hwnd,
                &util::to_wide("EDIT"),
                &util::to_wide(""),
                WS_CHILD | WS_VISIBLE | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | WS_VSCROLL
                    | ES_READONLY as u32,
                WS_EX_CLIENTEDGE,
                IDC_LOG,
            );
            c.start = create_ctl(
                hwnd,
                &util::to_wide("BUTTON"),
                &util::to_wide("開始護欄"),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
                0,
                1,
            );
            c.stop = create_ctl(
                hwnd,
                &util::to_wide("BUTTON"),
                &util::to_wide("停止護欄"),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
                0,
                2,
            );
            c.settings = create_ctl(
                hwnd,
                &util::to_wide("BUTTON"),
                &util::to_wide("設定"),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
                0,
                3,
            );
            c.openlog = create_ctl(
                hwnd,
                &util::to_wide("BUTTON"),
                &util::to_wide("開啟 Log 檔"),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
                0,
                4,
            );
            c.openfolder = create_ctl(
                hwnd,
                &util::to_wide("BUTTON"),
                &util::to_wide("開啟資料夾"),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
                0,
                5,
            );
            let dpi = GetDpiForWindow(hwnd);
            recreate_fonts(c, dpi);
            layout_main(c, dpi);
            // 允許較低權限實例送來的激活訊息（單例聚焦用）；TaskbarCreated 廣播也要放行
            ChangeWindowMessageFilterEx(hwnd, activate_message_id(), MSGFLT_ALLOW, std::ptr::null_mut());
            ChangeWindowMessageFilterEx(hwnd, taskbar_message_id(), MSGFLT_ALLOW, std::ptr::null_mut());
            SetTimer(hwnd, IDT_TIMER as usize, 1000, None);
            load_log_history(c);
            update_status(c);
            0
        }
        WM_TIMER => {
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MainCtx;
            if !ctx.is_null() {
                update_status(&mut *ctx);
            }
            0
        }
        WM_COMMAND => {
            let id = (wparam as u32) & 0xFFFF;
            let notif = ((wparam as u32) >> 16) & 0xFFFF;
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MainCtx;
            // 只接受按鈕點擊（BN_CLICKED=0）；忽略 EDIT/STATIC 控制項的 EN_* 通知，
            // 否則寫 log → EDIT 變動 → EN_CHANGE → 又寫 log 會形成無限循環
            if notif != 0 || ctx.is_null() {
                return 0;
            }
            let c = &mut *ctx;
            match id {
                    1 => guard_start(&c.app),
                    2 => guard_stop(&c.app),
                    3 => {
                        ui_settings::show_settings(c.app.clone(), hwnd);
                    }
                    4 => {
                        util::open_with_shell(&c.app.log_path, None);
                    }
                    5 => {
                        let folder = std::path::Path::new(&c.app.log_path)
                            .parent()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();
                        util::open_with_shell("explorer.exe", Some(&format!("/select,\"{}\"", folder)));
                    }
                    _ => {}
                }
            0
        }
        WM_SIZE => {
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MainCtx;
            if !ctx.is_null() {
                let dpi = GetDpiForWindow(hwnd);
                layout_main(&mut *ctx, dpi);
            }
            0
        }
        WM_GETMINMAXINFO => {
            // 主視窗最小尺寸（對齊 C# 版 MinimumSize 560x400，隨 DPI 縮放）
            let mm = lparam as *mut MINMAXINFO;
            if !mm.is_null() {
                let dpi = GetDpiForWindow(hwnd);
                let scale = dpi as f32 / 96.0;
                let s = |v: i32| (v as f32 * scale).round() as i32;
                let mut wr: RECT = std::mem::zeroed();
                wr.right = s(560);
                wr.bottom = s(400);
                let style = (WS_OVERLAPPEDWINDOW) as u32;
                AdjustWindowRectEx(&mut wr, style, 0, 0);
                (*mm).ptMinTrackSize.x = wr.right - wr.left;
                (*mm).ptMinTrackSize.y = wr.bottom - wr.top;
            }
            0
        }
        WM_EXITSIZEMOVE => {
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MainCtx;
            if !ctx.is_null() {
                save_geometry(&*ctx);
            }
            0
        }
        WM_DPICHANGED => {
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MainCtx;
            if !ctx.is_null() {
                let c = &mut *ctx;
                let rc = lparam as *const RECT;
                if !rc.is_null() {
                    let rect = &*rc;
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                let dpi = GetDpiForWindow(hwnd);
                recreate_fonts(c, dpi);
                layout_main(c, dpi);
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MainCtx;
            if !ctx.is_null() {
                let c = &*ctx;
                let ctl = lparam as HWND;
                let hdc = wparam as windows_sys::Win32::Graphics::Gdi::HDC;
                if ctl == c.status || ctl == c.admin {
                    SetTextColor(
                        hdc,
                        if ctl == c.status {
                            c.status_color.load(Ordering::SeqCst)
                        } else {
                            COLOR_ORANGE
                        },
                    );
                    // 實心背景刷：文字變短時舊像素才會被擦掉（HOLLOW+透明會殘影重疊）
                    SetBkMode(hdc, TRANSPARENT as i32);
                    return GetSysColorBrush(COLOR_WINDOW) as LRESULT;
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_TRAY => {
            let e = (lparam as u32) & 0xFFFF;
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MainCtx;
            if !ctx.is_null() {
                let c = &mut *ctx;
                match e {
                    WM_LBUTTONDBLCLK => {
                        ShowWindow(hwnd, SW_SHOW);
                        SetForegroundWindow(hwnd);
                    }
                    WM_RBUTTONUP => {
                        let hmenu = CreatePopupMenu();
                        let o = util::to_wide("開啟主視窗");
                        AppendMenuW(hmenu, MF_STRING, MENU_OPEN as usize, o.as_ptr());
                        AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
                        let s = util::to_wide("設定");
                        AppendMenuW(hmenu, MF_STRING, MENU_SETTINGS as usize, s.as_ptr());
                        let e2 = util::to_wide("結束");
                        AppendMenuW(hmenu, MF_STRING, MENU_EXIT as usize, e2.as_ptr());
                        let mut pt: POINT = std::mem::zeroed();
                        GetCursorPos(&mut pt);
                        SetForegroundWindow(hwnd);
                        let cmd = TrackPopupMenu(
                            hmenu,
                            TPM_RIGHTBUTTON | TPM_RETURNCMD,
                            pt.x,
                            pt.y,
                            0,
                            hwnd,
                            std::ptr::null(),
                        );
                        DestroyMenu(hmenu);
                        match cmd as usize {
                            MENU_OPEN => {
                                ShowWindow(hwnd, SW_SHOW);
                                SetForegroundWindow(hwnd);
                            }
                            MENU_SETTINGS => {
                                ui_settings::show_settings(c.app.clone(), hwnd);
                            }
                            MENU_EXIT => exit_app(c),
                            _ => {}
                        }
                        // 標準修法：讓托盤選單在點擊別處時確實收起
                        PostMessageW(hwnd, 0, 0, 0);
                    }
                    _ => {}
                }
            }
            0
        }
        WM_CLOSE => {
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MainCtx;
            if !ctx.is_null() {
                let c = &mut *ctx;
                if !c.app.exiting.load(Ordering::SeqCst) {
                    save_geometry(c);
                    ShowWindow(hwnd, SW_HIDE);
                    tray_balloon(hwnd, "System-OOM-Guard", "已最小化到系統托盤，護欄仍在運行。");
                    return 0;
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_POWERBROADCAST => {
            // 喚醒：記憶體樣貌與時間基準都已失效。
            // 重置滯回旗標與冷卻起點，讓下一輪週期從乾淨狀態重新評估；
            // 若壓力是真實的，會在一個輪詢間隔內自然重新觸發
            if wparam == PBT_APMRESUMEAUTOMATIC as usize {
                let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MainCtx;
                if !ctx.is_null() {
                    let c = &mut *ctx;
                    c.app.pressured.store(false, Ordering::SeqCst);
                    *c.app.last_kill.lock().unwrap() = SystemTime::now();
                    c.app.snapshot.lock().unwrap().pressured = false;
                    crate::util::append_log(&c.app, "[System] 系統喚醒，已重置壓力狀態與冷卻");
                }
                1 // WM_POWERBROADCAST 處理完應回傳 TRUE
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_DESTROY => {
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MainCtx;
            if !ctx.is_null() {
                let c = &mut *ctx;
                remove_tray(hwnd);
                KillTimer(hwnd, IDT_TIMER as usize);
                if !c.font_status.is_null() {
                    DeleteObject(c.font_status);
                }
                if !c.font_log.is_null() {
                    DeleteObject(c.font_log);
                }
                if !c.font_btn.is_null() {
                    DeleteObject(c.font_btn);
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ctx));
            }
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}