use std::ffi::c_void;
use std::sync::Arc;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, CLEARTYPE_QUALITY, COLOR_BTNFACE, DEFAULT_CHARSET, DEFAULT_PITCH,
    FF_DONTCARE, FW_NORMAL, HBRUSH, HFONT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::BST_CHECKED;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, VK_ESCAPE};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetClientRect, GetMessageW, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, IsDialogMessageW, MINMAXINFO, MoveWindow, RegisterClassExW, SendMessageW,
    SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage, BM_GETCHECK,
    BM_SETCHECK, BS_AUTOCHECKBOX, BS_PUSHBUTTON, CREATESTRUCTW, CW_USEDEFAULT, ES_AUTOVSCROLL,
    ES_MULTILINE, ES_NUMBER, ES_WANTRETURN, GWLP_USERDATA, HMENU, MSG, SW_SHOW, SWP_NOACTIVATE,
    SWP_NOZORDER,
    WS_BORDER, WS_CAPTION, WS_CHILD, WS_EX_CLIENTEDGE, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP,
    WS_THICKFRAME, WS_VISIBLE, WS_VSCROLL, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_DPICHANGED,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_SETFONT, WM_SIZE,
};

use crate::autostart;
use crate::state::App;
use crate::util;

const SETTINGS_CLASS: &str = "OOMGuardSettingsWnd";
const SETTINGS_STYLE: u32 = (WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME) as u32;

fn settings_client_min(scale: f32) -> (i32, i32) {
    let s = |v: i32| (v as f32 * scale).round() as i32;
    (s(536), s(580))
}

unsafe fn settings_window_min(scale: f32) -> (i32, i32) {
    let (cw, chh) = settings_client_min(scale);
    let mut wr: RECT = std::mem::zeroed();
    wr.right = cw;
    wr.bottom = chh;
    AdjustWindowRectEx(&mut wr, SETTINGS_STYLE, 0, 0);
    (wr.right - wr.left, wr.bottom - wr.top)
}

const IDC_TRIGGER: usize = 1;
const IDC_RECOVER: usize = 2;
const IDC_INTERVAL: usize = 3;
const IDC_MAXKILL: usize = 4;
const IDC_COOLDOWN: usize = 5;
const IDC_AI: usize = 6;
const IDC_PROTECTED: usize = 7;
const IDC_AUTOSTART: usize = 8;
const IDC_AUTOGUARD: usize = 11;
const IDC_KILLTREE: usize = 12;
const IDC_SAVE: usize = 9;
const IDC_CANCEL: usize = 10;

struct SettingsCtx {
    app: Arc<App>,
    closed: bool,
    hwnd: HWND,
    font: HFONT,
    trigger: HWND,
    recover: HWND,
    interval: HWND,
    maxkill: HWND,
    cooldown: HWND,
    ai: HWND,
    protected: HWND,
    autostart: HWND,
    autoguard: HWND,
    killtree: HWND,
    l_trigger: HWND,
    l_recover: HWND,
    l_interval: HWND,
    l_maxkill: HWND,
    l_cooldown: HWND,
    l_ai: HWND,
    l_protected: HWND,
    save_btn: HWND,
    cancel_btn: HWND,
}

pub unsafe fn show_settings(app: Arc<App>, parent: HWND) {
    register_settings_class();

    let ctx = Box::new(SettingsCtx {
        app,
        closed: false,
        hwnd: std::ptr::null_mut(),
        font: std::ptr::null_mut(),
        trigger: std::ptr::null_mut(),
        recover: std::ptr::null_mut(),
        interval: std::ptr::null_mut(),
        maxkill: std::ptr::null_mut(),
        cooldown: std::ptr::null_mut(),
        ai: std::ptr::null_mut(),
        protected: std::ptr::null_mut(),
        autostart: std::ptr::null_mut(),
        autoguard: std::ptr::null_mut(),
        killtree: std::ptr::null_mut(),
        l_trigger: std::ptr::null_mut(),
        l_recover: std::ptr::null_mut(),
        l_interval: std::ptr::null_mut(),
        l_maxkill: std::ptr::null_mut(),
        l_cooldown: std::ptr::null_mut(),
        l_ai: std::ptr::null_mut(),
        l_protected: std::ptr::null_mut(),
        save_btn: std::ptr::null_mut(),
        cancel_btn: std::ptr::null_mut(),
    });
    let ctx_ptr = Box::into_raw(ctx);

    let hwnd = CreateWindowExW(
        0,
        util::to_wide(SETTINGS_CLASS).as_ptr(),
        util::to_wide("設定").as_ptr(),
        SETTINGS_STYLE,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        540,
        560,
        parent,
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        ctx_ptr as *const c_void,
    );
    if hwnd.is_null() {
        drop(Box::from_raw(ctx_ptr));
        return;
    }
    (*ctx_ptr).hwnd = hwnd;
    // 標準 modal：停用擁有者視窗，防止重複開啟設定或在對話框開著時關閉主視窗
    EnableWindow(parent, 0);
    let dpi = GetDpiForWindow(hwnd);
    let scale = dpi as f32 / 96.0;
    let (ww, wh) = settings_window_min(scale);
    center_on_parent(hwnd, parent, ww, wh);
    ShowWindow(hwnd, SW_SHOW);

    let closed_ptr = &mut (*ctx_ptr).closed as *mut bool;
    let mut msg: MSG = std::mem::zeroed();
    loop {
        if unsafe { *closed_ptr } {
            break;
        }
        let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
        if r == 0 || r == -1 {
            break;
        }
        if IsDialogMessageW(hwnd, &msg) == 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    EnableWindow(parent, 1);

    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    drop(Box::from_raw(ctx_ptr));
}

unsafe fn register_settings_class() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let mut wc: windows_sys::Win32::UI::WindowsAndMessaging::WNDCLASSEXW = std::mem::zeroed();
        wc.cbSize = std::mem::size_of::<windows_sys::Win32::UI::WindowsAndMessaging::WNDCLASSEXW>()
            as u32;
        wc.lpfnWndProc = Some(settings_wndproc);
        wc.hInstance = GetModuleHandleW(std::ptr::null());
        wc.hbrBackground = (COLOR_BTNFACE as isize + 1) as HBRUSH;
        let cls = util::to_wide(SETTINGS_CLASS);
        wc.lpszClassName = cls.as_ptr();
        RegisterClassExW(&wc);
    });
}

unsafe fn center_on_parent(hwnd: HWND, parent: HWND, w: i32, h: i32) {
    let mut pr: RECT = std::mem::zeroed();
    let mut wr: RECT = std::mem::zeroed();
    GetWindowRect(hwnd, &mut wr);
    GetWindowRect(parent, &mut pr);
    let pw = pr.right - pr.left;
    let ph = pr.bottom - pr.top;
    let x = pr.left + (pw - w) / 2;
    let y = pr.top + (ph - h) / 2;
    SetWindowPos(hwnd, std::ptr::null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
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

unsafe fn create_font(px: i32, face: &[u16]) -> HFONT {
    CreateFontW(
        -px,
        0,
        0,
        0,
        FW_NORMAL as i32,
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

unsafe fn recreate_font(c: &mut SettingsCtx, dpi: u32) {
    let scale = dpi as f32 / 96.0;
    let px = (12.0 * scale).round() as i32;
    let font = create_font(px, &util::to_wide("Segoe UI"));
    if !c.font.is_null() {
        DeleteObject(c.font);
    }
    c.font = font;
    let hwnds = [
        c.trigger,
        c.recover,
        c.interval,
        c.maxkill,
        c.cooldown,
        c.ai,
        c.protected,
        c.autostart,
        c.autoguard,
        c.killtree,
        c.l_trigger,
        c.l_recover,
        c.l_interval,
        c.l_maxkill,
        c.l_cooldown,
        c.l_ai,
        c.l_protected,
        c.save_btn,
        c.cancel_btn,
    ];
    for h in hwnds {
        if !h.is_null() {
            SendMessageW(h, WM_SETFONT, c.font as WPARAM, 1);
        }
    }
}

unsafe fn layout_settings(c: &mut SettingsCtx, dpi: u32) {
    let hwnd = c.hwnd;
    let scale = dpi as f32 / 96.0;
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rc);
    let ch = rc.bottom - rc.top;
    let s = |v: i32| (v as f32 * scale).round() as i32;

    let lx = s(16);
    let label_w = s(190);
    let ex = lx + s(202);
    let ew = s(306);
    let step = s(36);
    let edit_h = s(22);

    let mut y = s(8);
    for (lbl, edit, hgt) in [
        (c.l_trigger, c.trigger, edit_h),
        (c.l_recover, c.recover, edit_h),
        (c.l_interval, c.interval, edit_h),
        (c.l_maxkill, c.maxkill, edit_h),
        (c.l_cooldown, c.cooldown, edit_h),
        (c.l_ai, c.ai, edit_h),
    ] {
        MoveWindow(lbl, lx, y, label_w, edit_h, 1);
        MoveWindow(edit, ex, y, ew, hgt, 1);
        y += step;
    }
    let proto_h = s(200);
    MoveWindow(c.l_protected, lx, y, label_w, edit_h, 1);
    MoveWindow(c.protected, ex, y, ew, proto_h, 1);
    y += proto_h + s(8);
    MoveWindow(c.autostart, ex, y, ew, s(24), 1);
    y += s(30);
    MoveWindow(c.autoguard, ex, y, ew, s(24), 1);
    y += s(30);
    MoveWindow(c.killtree, ex, y, ew, s(24), 1);

    let btn_y = ch - s(38);
    let btn_h = s(30);
    let btn_w = s(90);
    let save_x = lx + ew - btn_w;
    let cancel_x = save_x - s(8) - btn_w;
    MoveWindow(c.save_btn, save_x, btn_y, btn_w, btn_h, 1);
    MoveWindow(c.cancel_btn, cancel_x, btn_y, btn_w, btn_h, 1);
}

unsafe fn get_edit_text(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len + 1) as usize];
    GetWindowTextW(hwnd, buf.as_mut_ptr(), len + 1);
    util::from_wide(buf.as_ptr())
}

unsafe fn set_value(hwnd: HWND, v: u32) {
    SetWindowTextW(hwnd, util::to_wide(&v.to_string()).as_ptr());
}

unsafe fn save_settings(c: &mut SettingsCtx) {
    let parse = |hwnd: HWND, default: u32| -> u32 {
        get_edit_text(hwnd).trim().parse::<u32>().unwrap_or(default)
    };
    let clamp = |v: u32, lo: u32, hi: u32| v.clamp(lo, hi);

    let trigger = clamp(parse(c.trigger, 95), 1, 100);
    // 恢復門檻不得高於觸發門檻，維持滯回區間有效
    let recover = clamp(parse(c.recover, 85), 1, 100).min(trigger);
    let interval = clamp(parse(c.interval, 5), 1, 3600);
    let maxkill = clamp(parse(c.maxkill, 3), 1, 20);
    let cooldown = clamp(parse(c.cooldown, 30), 0, 86400);

    let ai = get_edit_text(c.ai).trim().to_string();
    let protected_text = get_edit_text(c.protected);
    let protected: Vec<String> = protected_text
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let auto = SendMessageW(c.autostart, BM_GETCHECK, 0, 0) as u32 == BST_CHECKED;
    let auto_guard = SendMessageW(c.autoguard, BM_GETCHECK, 0, 0) as u32 == BST_CHECKED;
    let kill_tree = SendMessageW(c.killtree, BM_GETCHECK, 0, 0) as u32 == BST_CHECKED;

    let new_cfg = {
        let mut guard = c.app.config.lock().unwrap();
        guard.trigger_used_percent = trigger;
        guard.recover_used_percent = recover;
        guard.interval_sec = interval;
        guard.max_kill_per_cycle = maxkill;
        guard.cooldown_sec = cooldown;
        guard.ai_process_pattern = ai;
        guard.protected_names = protected;
        guard.auto_start = auto;
        guard.auto_guard = auto_guard;
        guard.kill_tree = kill_tree;
        guard.clone()
    };
    new_cfg.save(&c.app.config_path);
    *c.app.config.lock().unwrap() = new_cfg.clone();

    if new_cfg.auto_start {
        autostart::enable(&c.app.exe_path);
    } else {
        autostart::disable();
    }

    util::append_log(&c.app, "設定已儲存");
}

unsafe extern "system" fn settings_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let cs = lparam as *const CREATESTRUCTW;
            let ctx = (*cs).lpCreateParams as *mut SettingsCtx;
            if ctx.is_null() {
                return -1 as LRESULT;
            }
            (*ctx).hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, ctx as isize);
            let c = &mut *ctx;

            let cfg = c.app.config.lock().unwrap();

            let edit_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_NUMBER as u32 | WS_BORDER;
            c.trigger = create_ctl(
                hwnd,
                &util::to_wide("EDIT"),
                &util::to_wide(""),
                edit_style,
                0,
                IDC_TRIGGER,
            );
            c.recover = create_ctl(
                hwnd,
                &util::to_wide("EDIT"),
                &util::to_wide(""),
                edit_style,
                0,
                IDC_RECOVER,
            );
            c.interval = create_ctl(
                hwnd,
                &util::to_wide("EDIT"),
                &util::to_wide(""),
                edit_style,
                0,
                IDC_INTERVAL,
            );
            c.maxkill = create_ctl(
                hwnd,
                &util::to_wide("EDIT"),
                &util::to_wide(""),
                edit_style,
                0,
                IDC_MAXKILL,
            );
            c.cooldown = create_ctl(
                hwnd,
                &util::to_wide("EDIT"),
                &util::to_wide(""),
                edit_style,
                0,
                IDC_COOLDOWN,
            );
            c.ai = create_ctl(
                hwnd,
                &util::to_wide("EDIT"),
                &util::to_wide(""),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER,
                0,
                IDC_AI,
            );
            c.protected = create_ctl(
                hwnd,
                &util::to_wide("EDIT"),
                &util::to_wide(""),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32
                    | ES_WANTRETURN as u32 | WS_VSCROLL | WS_BORDER,
                WS_EX_CLIENTEDGE,
                IDC_PROTECTED,
            );
            c.autostart = create_ctl(
                hwnd,
                &util::to_wide("BUTTON"),
                &util::to_wide("開機自動啟動（登入觸發排程任務）"),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                0,
                IDC_AUTOSTART,
            );
            c.autoguard = create_ctl(
                hwnd,
                &util::to_wide("BUTTON"),
                &util::to_wide("程式啟動後自動啟用護欄"),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                0,
                IDC_AUTOGUARD,
            );
            c.killtree = create_ctl(
                hwnd,
                &util::to_wide("BUTTON"),
                &util::to_wide("終止時連帶殺死整棵子進程樹（預設）"),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                0,
                IDC_KILLTREE,
            );

            let label_style = WS_CHILD | WS_VISIBLE;
            c.l_trigger = create_ctl(
                hwnd,
                &util::to_wide("STATIC"),
                &util::to_wide("觸發使用率 (%)"),
                label_style,
                0,
                20,
            );
            c.l_recover = create_ctl(
                hwnd,
                &util::to_wide("STATIC"),
                &util::to_wide("恢復使用率 (%)"),
                label_style,
                0,
                21,
            );
            c.l_interval = create_ctl(
                hwnd,
                &util::to_wide("STATIC"),
                &util::to_wide("檢查間隔 (秒)"),
                label_style,
                0,
                22,
            );
            c.l_maxkill = create_ctl(
                hwnd,
                &util::to_wide("STATIC"),
                &util::to_wide("每輪最多終止數"),
                label_style,
                0,
                23,
            );
            c.l_cooldown = create_ctl(
                hwnd,
                &util::to_wide("STATIC"),
                &util::to_wide("冷卻時間 (秒)"),
                label_style,
                0,
                24,
            );
            c.l_ai = create_ctl(
                hwnd,
                &util::to_wide("STATIC"),
                &util::to_wide("AI 進程名稱特徵"),
                label_style,
                0,
                25,
            );
            c.l_protected = create_ctl(
                hwnd,
                &util::to_wide("STATIC"),
                &util::to_wide("受保護進程（每行一個）"),
                label_style,
                0,
                26,
            );

            let btn_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32;
            c.save_btn = create_ctl(
                hwnd,
                &util::to_wide("BUTTON"),
                &util::to_wide("儲存"),
                btn_style,
                0,
                IDC_SAVE,
            );
            c.cancel_btn = create_ctl(
                hwnd,
                &util::to_wide("BUTTON"),
                &util::to_wide("取消"),
                btn_style,
                0,
                IDC_CANCEL,
            );

            set_value(c.trigger, cfg.trigger_used_percent);
            set_value(c.recover, cfg.recover_used_percent);
            set_value(c.interval, cfg.interval_sec);
            set_value(c.maxkill, cfg.max_kill_per_cycle);
            set_value(c.cooldown, cfg.cooldown_sec);
            SetWindowTextW(c.ai, util::to_wide(&cfg.ai_process_pattern).as_ptr());
            SetWindowTextW(c.protected, util::to_wide(&cfg.protected_names.join("\r\n")).as_ptr());
            SendMessageW(
                c.autostart,
                BM_SETCHECK,
                if cfg.auto_start { BST_CHECKED as usize } else { 0 },
                0,
            );
            SendMessageW(
                c.autoguard,
                BM_SETCHECK,
                if cfg.auto_guard { BST_CHECKED as usize } else { 0 },
                0,
            );
            SendMessageW(
                c.killtree,
                BM_SETCHECK,
                if cfg.kill_tree { BST_CHECKED as usize } else { 0 },
                0,
            );
            drop(cfg);

            let dpi = GetDpiForWindow(hwnd);
            recreate_font(c, dpi);
            layout_settings(c, dpi);
            0
        }
        WM_COMMAND => {
            let id = (wparam as u32) & 0xFFFF;
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsCtx;
            if !ctx.is_null() {
                let c = &mut *ctx;
                let id_u32 = id;
                if id_u32 == IDC_SAVE as u32 {
                    save_settings(c);
                    DestroyWindow(hwnd);
                } else if id_u32 == IDC_CANCEL as u32 {
                    DestroyWindow(hwnd);
                }
            }
            0
        }
        WM_KEYDOWN => {
            if wparam as u16 == VK_ESCAPE {
                DestroyWindow(hwnd);
                return 0;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DPICHANGED => {
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsCtx;
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
                recreate_font(c, dpi);
                layout_settings(c, dpi);
            }
            0
        }
        WM_SIZE => {
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsCtx;
            if !ctx.is_null() {
                let dpi = GetDpiForWindow(hwnd);
                layout_settings(&mut *ctx, dpi);
            }
            0
        }
        WM_GETMINMAXINFO => {
            let mm = lparam as *mut MINMAXINFO;
            if !mm.is_null() {
                let dpi = GetDpiForWindow(hwnd);
                let (mw, mh) = settings_window_min(dpi as f32 / 96.0);
                (*mm).ptMinTrackSize.x = mw;
                (*mm).ptMinTrackSize.y = mh;
            }
            0
        }
        WM_DESTROY => {
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsCtx;
            if !ctx.is_null() {
                let c = &mut *ctx;
                c.closed = true;
                if !c.font.is_null() {
                    DeleteObject(c.font);
                }
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}