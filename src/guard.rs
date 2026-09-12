use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LPARAM, HWND, UNICODE_STRING,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, SE_DEBUG_NAME, SE_PRIVILEGE_ENABLED,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{
    GetProcessImageFileNameW, GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcess, OpenProcessToken, TerminateProcess,
    WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible};

use crate::state::App;
use crate::util;

/// SYNCHRONIZE 存取權：WaitForSingleObject 需要此權限才能等待 handle
const SYNCHRONIZE: u32 = 0x0010_0000;
const STILL_ACTIVE: u32 = 259;
/// 子進程樹的影像名稱緩衝大小（字元數）
const IMAGE_NAME_BUF: usize = 1024;

static DEBUG_PRIV: AtomicBool = AtomicBool::new(false);

pub fn enable_debug_privilege() -> Result<(), u32> {
    if DEBUG_PRIV.load(Ordering::SeqCst) {
        return Ok(());
    }
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        ) == 0
        {
            return Err(GetLastError());
        }
        let mut tp: TOKEN_PRIVILEGES = std::mem::zeroed();
        tp.PrivilegeCount = 1;
        tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
        if LookupPrivilegeValueW(std::ptr::null(), SE_DEBUG_NAME, &mut tp.Privileges[0].Luid) == 0 {
            let e = GetLastError();
            CloseHandle(token);
            return Err(e);
        }
        let ok = AdjustTokenPrivileges(token, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut());
        CloseHandle(token);
        if ok == 0 {
            return Err(GetLastError());
        }
        // AdjustTokenPrivileges 成功不代表已指派：1300 = ERROR_NOT_ALL_ASSIGNED
        let err = GetLastError();
        if err == 0 {
            DEBUG_PRIV.store(true, Ordering::SeqCst);
            Ok(())
        } else {
            Err(err)
        }
    }
}

pub fn memory_status() -> (u64, u64) {
    unsafe {
        let mut m: MEMORYSTATUSEX = std::mem::zeroed();
        m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut m) != 0 {
            (m.ullTotalPhys / 1_048_576, m.ullAvailPhys / 1_048_576)
        } else {
            (0, 0)
        }
    }
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lp: LPARAM) -> i32 {
    let set = lp as *mut HashSet<u32>;
    if set.is_null() {
        return 0;
    }
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if IsWindowVisible(hwnd) != 0 {
        (*set).insert(pid);
    }
    1
}

fn windowed_pids() -> HashSet<u32> {
    let mut set = HashSet::new();
    unsafe {
        EnumWindows(Some(enum_proc), &mut set as *mut HashSet<u32> as isize);
    }
    set
}

fn private_bytes(pid: u32) -> u64 {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return 0;
        }
        let mut mc: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        mc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let ok = GetProcessMemoryInfo(
            h,
            &mut mc as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            mc.cb,
        );
        CloseHandle(h);
        if ok == 0 {
            0
        } else {
            mc.PrivateUsage as u64
        }
    }
}

/// SYSTEM_PROCESS_INFORMATION 的固定欄位（其後為變長執行緒陣列，僅以 NextEntryOffset 走訪）。
/// PrivatePageCount ≡ GetProcessMemoryInfo 的 PrivateUsage（私有提交記憶體）。
#[repr(C)]
struct SysProcInfo {
    next_entry_offset: u32,
    number_of_threads: u32,
    working_set_private_size: i64,
    hard_fault_count: u32,
    number_of_threads_high_watermark: u32,
    cycle_time: u64,
    create_time: i64,
    user_time: i64,
    kernel_time: i64,
    image_name: UNICODE_STRING,
    base_priority: i32,
    unique_process_id: usize,
    inherited_from_unique_process_id: usize,
    handle_count: u32,
    session_id: u32,
    page_directory_base: usize,
    peak_virtual_size: usize,
    virtual_size: usize,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_page_count: usize,
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQuerySystemInformation(
        class: u32,
        info: *mut c_void,
        len: u32,
        ret_len: *mut u32,
    ) -> i32;
}

/// 單一系統呼叫取得全系統 PID→私有記憶體對照表，取代對每個候選各開一次
/// OpenProcess + GetProcessMemoryInfo（典型桌面環境可省下數百次系統呼叫）。
/// 失敗時回傳 None，由呼叫端退回原本的逐進程查詢。
fn private_bytes_map() -> Option<HashMap<u32, u64>> {
    const STATUS_SUCCESS: i32 = 0;
    const STATUS_INFO_LENGTH_MISMATCH: i32 = -1073741820; // 0xC0000004
    // 上限刻意保守：記憶體耗盡時（正是護欄要介入的時機），過大的單一配置會讓
    // 配置失敗走 handle_alloc_error → abort（整程式閃退）。寧可在這個上限內失敗
    // 後退回逐進程查詢（每筆只配置極小緩衝，記憶體吃緊時更容易成功），
    // 也不冒 OOM-abort 風險。32 MiB 涵蓋一般桌面環境綽綽有餘。
    const MAX_BYTES: usize = 32 << 20;

    let mut len_bytes: usize = 1 << 20; // 1 MiB 起步
    for _ in 0..6 {
        // 以 Vec<u64> 保證 8 位元組對齊；用 try_reserve_exact 讓配置失敗時回 None
        // （vec![] 巨集在失敗時會直接 handle_alloc_error abort，不可攔截）。
        let units = len_bytes / 8;
        let mut buf: Vec<u64> = Vec::new();
        if buf.try_reserve_exact(units).is_err() {
            return None;
        }
        buf.resize(units, 0);
        let mut ret: u32 = 0;
        let status = unsafe {
            NtQuerySystemInformation(5, buf.as_mut_ptr().cast(), (units * 8) as u32, &mut ret)
        };
        if status == STATUS_SUCCESS {
            let total = units * 8;
            let esize = std::mem::size_of::<SysProcInfo>();
            // 一次預留到「最多可能筆數」的上界：走訪期間就不會再 realloc，
            // 唯一可能失敗的配置即此處，失敗即回退 None（避免走訪中 insert 才 abort）。
            let mut map: HashMap<u32, u64> = HashMap::new();
            if map.try_reserve(total / esize + 1).is_err() {
                return None;
            }
            unsafe {
                let mut off = 0usize;
                while off + esize <= total {
                    let e = &*((buf.as_ptr() as *const u8).add(off) as *const SysProcInfo);
                    map.insert(e.unique_process_id as u32, e.private_page_count as u64);
                    if e.next_entry_offset == 0 {
                        break;
                    }
                    off += e.next_entry_offset as usize;
                }
            }
            return Some(map);
        }
        if status != STATUS_INFO_LENGTH_MISMATCH {
            return None;
        }
        len_bytes = if ret as usize > len_bytes { ret as usize } else { len_bytes * 2 };
        if len_bytes > MAX_BYTES {
            return None;
        }
    }
    None
}

fn process_list() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut pe);
        while ok != 0 {
            let name = util::from_wide(pe.szExeFile.as_ptr());
            let pid = pe.th32ProcessID;
            out.push((pid, name));
            ok = Process32NextW(snap, &mut pe);
        }
        CloseHandle(snap);
    }
    out
}

/// 探測進程是否仍存活。OpenProcess 失敗（多半 ERROR_INVALID_PARAMETER，代表 PID 已不存在）
/// 或退出碼非 STILL_ACTIVE 都視為已結束。
fn pid_alive(pid: u32) -> bool {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        ok != 0 && code == STILL_ACTIVE
    }
}

/// 進程終止結果：
/// - `Killed`：已確認結束（TerminateProcess 或 taskkill 成功）
/// - `AlreadyDead`：探測時已不在（zombie 或 PID 已消失），無需再動手
/// - `Pending`：終止要求已送出但仍在拆除中（不阻塞等待，由後續輪詢確認）
/// - `Failed`：所有手段皆失敗
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KillResult {
    Killed,
    AlreadyDead,
    Pending,
    Failed,
}

/// 對單一 PID 直接終止（不涉及子進程樹）。
/// 回傳：
/// - `Killed`：已確認結束
/// - `AlreadyDead`：探測時已不在
/// - `Pending`：終止要求已送出，仍在拆除中（由後續輪詢確認）
/// - `Failed`：直接終止失敗（呼叫端決定是否升級為樹狀終止）
fn terminate_direct<F>(pid: u32, log: &F) -> KillResult
where
    F: Fn(&str),
{
    unsafe {
        let h = OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
            0,
            pid,
        );
        if h.is_null() {
            log(&format!("OpenProcess(PID {pid}) 失敗：錯誤碼 {}", GetLastError()));
            return KillResult::Failed;
        }
        let r = TerminateProcess(h, 1);
        let err = GetLastError();
        if r != 0 {
            // TerminateProcess 為非同步完成。等待 2 秒：
            // 已結束 → Killed；逾時但已死 → Killed；
            // 仍存活 → 不繼續阻塞，回 Pending 讓輪詢確認（大型進程拆除可達數秒，
            // 但護欄執行緒不該被綁住 10 秒）
            if WaitForSingleObject(h, 2_000) == WAIT_OBJECT_0 {
                CloseHandle(h);
                return KillResult::Killed;
            }
            CloseHandle(h);
            if !pid_alive(pid) {
                log(&format!("PID {pid}：等待逾時，複查確認已結束"));
                return KillResult::Killed;
            }
            log(&format!("PID {pid}：終止要求已送出，等待結束"));
            return KillResult::Pending;
        }
        CloseHandle(h);
        if err == 5 {
            // 錯誤碼 5（ERROR_ACCESS_DENIED）：對一個「已終止但進程物件未釋放」的
            // zombie 再呼叫 TerminateProcess 就是回 5。先探測存活，已死就別再動手。
            if !pid_alive(pid) {
                log(&format!("PID {pid}：進程已結束（先前已終止）"));
                return KillResult::AlreadyDead;
            }
            log(&format!("PID {pid}：TerminateProcess 拒絕（錯誤碼 5）"));
        } else {
            log(&format!("PID {pid}：TerminateProcess 失敗（錯誤碼 {err}）"));
        }
        KillResult::Failed
    }
}

/// 取得進程的影像名稱（不含路徑）。PID 重用核對用。
/// 失敗（進程已消失或無權限）時回傳 None。
pub fn image_base_name(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; IMAGE_NAME_BUF];
        let n = GetProcessImageFileNameW(h, buf.as_mut_ptr(), IMAGE_NAME_BUF as u32);
        CloseHandle(h);
        if n == 0 {
            return None;
        }
        let len = (n as usize).min(IMAGE_NAME_BUF);
        // GetProcessImageFileNameW 回傳字元數，但不保證 null 終止；依實際長度截斷
        let s = String::from_utf16_lossy(&buf[..len]);
        // 回傳完整路徑（可能是 \Device\... 形式），取最後一段檔名
        Some(s.rsplit('\\').next().unwrap_or(&s).to_string())
    }
}

/// 遞迴收集 pid 的所有後代進程（不含 pid 自己），回傳 (子 PID, 影像名)。
/// 基於單次 CreateToolhelp32Snapshot：先建父子對應，再從目標向下 BFS。
/// 回傳順序：越深層的後代越靠前（先殺葉子，再殺靠近根的）。
pub fn collect_descendants(pid: u32) -> Vec<(u32, String)> {
    let mut children: HashMap<u32, Vec<(u32, String)>> = HashMap::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut pe);
        while ok != 0 {
            let name = util::from_wide(pe.szExeFile.as_ptr());
            let cpid = pe.th32ProcessID;
            let ppid = pe.th32ParentProcessID;
            children.entry(ppid).or_default().push((cpid, name));
            ok = Process32NextW(snap, &mut pe);
        }
        CloseHandle(snap);
    }

    let mut result = Vec::new();
    let mut stack = vec![pid];
    // 記錄已處理的 PID，防環狀引用／重複
    let mut visited = HashSet::new();
    visited.insert(pid);
    while let Some(p) = stack.pop() {
        if let Some(kids) = children.get(&p) {
            for (cpid, name) in kids {
                if visited.insert(*cpid) {
                    result.push((*cpid, name.clone()));
                    stack.push(*cpid);
                }
            }
        }
    }
    result
}

/// 終止選項。tree=true 時殺整棵樹（預設）；false 時只殺目標，失敗才升級。
#[derive(Default)]
pub struct KillOpts {
    /// true = 趁目標存活先快照並終止整棵子樹；false = 只殺目標，失敗才升級
    pub tree: bool,
    /// 使用者保護名單（已小寫、去 .exe）；用於過濾後代，避免連帶殺到受保護程序
    pub protected: Vec<String>,
}

/// 收集並終止 pid 的後代進程（不含 pid 自己）。
/// 內建最低保障名單與使用者保護名單一律跳過；並做 PID 重用影像名稱核對。
/// 回傳實際終止（含送出要求）的子進程數。
fn kill_descendants<F>(pid: u32, protected: &[String], log: &F) -> u32
where
    F: Fn(&str),
{
    let descendants = collect_descendants(pid);
    if descendants.is_empty() {
        return 0;
    }
    let me = std::process::id();
    let mut tree_killed = 0u32;
    for (cpid, name) in descendants {
        if cpid == me {
            continue;
        }
        let base = name.trim_end_matches(".exe").to_lowercase();
        if base.is_empty() {
            continue;
        }
        if FLOOR_PROTECTED.contains(&base.as_str()) || protected.contains(&base) {
            log(&format!("跳過受保護的子進程 {name} (PID {cpid})"));
            continue;
        }
        // PID 重用防護：以影像名稱核對，避免殺到被重用 PID 的無關進程
        if let Some(img) = image_base_name(cpid) {
            let actual = img.trim_end_matches(".exe").to_lowercase();
            if !actual.is_empty() && actual != base {
                log(&format!(
                    "PID {cpid} 影像名稱不符（期望 {name}，實際 {actual}），跳過（PID 重用防護）"
                ));
                continue;
            }
        }
        match terminate_direct(cpid, log) {
            KillResult::Killed => {
                log(&format!("已終止子進程 {name} (PID {cpid})"));
                tree_killed += 1;
            }
            KillResult::Pending => {
                log(&format!("已送出子進程終止要求 {name} (PID {cpid})"));
                tree_killed += 1;
            }
            KillResult::AlreadyDead => {
                log(&format!("子進程 {name} (PID {cpid}) 已結束"));
            }
            KillResult::Failed => {
                log(&format!("無法終止子進程 {name} (PID {cpid})"));
            }
        }
    }
    tree_killed
}

pub fn terminate_with<F>(pid: u32, opts: &KillOpts, log: F) -> KillResult
where
    F: Fn(&str),
{
    if opts.tree {
        // 樹狀預設：趁目標存活，先快照並終止整棵子樹，再終止目標本身。
        let tree_killed = kill_descendants(pid, &opts.protected, &log);
        match terminate_direct(pid, &log) {
            KillResult::Killed | KillResult::AlreadyDead => {
                if tree_killed > 0 {
                    log(&format!("已終止 {pid} 及其 {tree_killed} 個子進程"));
                }
                KillResult::Killed
            }
            KillResult::Pending => KillResult::Pending,
            KillResult::Failed => {
                log(&format!("PID {pid} 無法終止（已連帶處理 {tree_killed} 個子進程）"));
                KillResult::Failed
            }
        }
    } else {
        // 只殺目標；失敗才升級為連帶終止子樹。
        match terminate_direct(pid, &log) {
            KillResult::Killed | KillResult::AlreadyDead => return KillResult::Killed,
            KillResult::Pending => return KillResult::Pending,
            KillResult::Failed => {}
        }
        let tree_killed = kill_descendants(pid, &opts.protected, &log);
        if tree_killed == 0 {
            log(&format!("PID {pid} 無可終止的子進程（無子樹，或子進程均受保護／PID 重用）"));
            return KillResult::Failed;
        }
        // 子樹清完後，回頭再試目標本身（其子進程可能正握著它的資源）
        match terminate_direct(pid, &log) {
            KillResult::Killed | KillResult::AlreadyDead => {
                log(&format!("已終止 {pid} 及其 {tree_killed} 個子進程"));
                KillResult::Killed
            }
            KillResult::Pending => KillResult::Pending,
            KillResult::Failed => {
                log(&format!("PID {pid} 連帶子進程後仍無法終止"));
                KillResult::Failed
            }
        }
    }
}

fn terminate_pid(app: &App, pid: u32, protected: &[String]) -> KillResult {
    let tree = app.config.lock().unwrap().kill_tree;
    let opts = KillOpts { tree, protected: protected.to_vec() };
    terminate_with(pid, &opts, |msg| util::append_log(app, msg))
}

/// 內建最低保障名單：無論使用者怎麼清空設定，這些一律不可殺
const FLOOR_PROTECTED: &[&str] = &[
    "system", "idle", "registry", "smss", "csrss", "wininit", "winlogon",
    "services", "lsass", "svchost", "dwm", "explorer", "fontdrvhost",
    "audiodg", "memory",
];

/// 跳過最近 N 秒內已處理過的進程，避免 zombie（已終止但物件未釋放）被重複選中。
fn prune_recent(app: &App, window: std::time::Duration) {
    let mut rk = app.recent_kills.lock().unwrap();
    rk.retain(|_, t| t.elapsed() < window);
}

fn kill_top(app: &App, count: usize) -> u32 {
    let me = std::process::id();
    let windowed = windowed_pids();
    // 先清理過期紀錄，再取得「近期已處理」集合供候選過濾
    prune_recent(app, std::time::Duration::from_secs(30));
    let recent: std::collections::HashSet<u32> = {
        let rk = app.recent_kills.lock().unwrap();
        rk.keys().copied().collect()
    };

    let protected = {
        let cfg = app.config.lock().unwrap();
        cfg.protected_names
            .iter()
            .map(|s| s.trim().trim_end_matches(".exe").to_lowercase())
            .filter(|s| !s.is_empty())
            .collect::<Vec<String>>()
    };
    let tokens: Vec<String> = {
        let cfg = app.config.lock().unwrap();
        let pattern = cfg.ai_process_pattern.trim();
        pattern
            .split('|')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let pattern_empty = tokens.is_empty();

    // 先用名稱/視窗/「近期已處理」過濾，再以單次系統快照取得私有記憶體排名；
    // 快照失敗時退回逐候選 OpenProcess 查詢
    let mem_map = private_bytes_map();
    let list = process_list();
    let mut candidates: Vec<(u64, u32, String)> = list
        .into_iter()
        .filter(|(pid, name)| {
            // szExeFile 含 ".exe"；保護名單與特徵比對一律用去掉副檔名的主檔名
            let base = name.strip_suffix(".exe").unwrap_or(name).to_lowercase();
            *pid != me
                && !base.is_empty()
                && !FLOOR_PROTECTED.contains(&base.as_str())
                && !protected.contains(&base)
                && !recent.contains(pid)
                && (windowed.contains(pid) || pattern_empty || tokens.iter().any(|t| base.contains(t)))
        })
        .map(|(pid, name)| {
            let privb = match &mem_map {
                Some(m) => m.get(&pid).copied().unwrap_or(0),
                None => private_bytes(pid),
            };
            (privb, pid, name)
        })
        .collect();

    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    candidates.truncate(count);

    let mut killed = 0u32;
    for (privb, pid, name) in candidates {
        let mb = privb / 1_048_576;
        match terminate_pid(app, pid, &protected) {
            KillResult::Killed => {
                util::append_log(app, &format!("已終止 {name} (PID {pid})，私有記憶體 {mb} MB"));
                app.recent_kills.lock().unwrap().insert(pid, std::time::Instant::now());
                killed += 1;
            }
            KillResult::AlreadyDead => {
                util::append_log(app, &format!("{name} (PID {pid}) 已結束，跳過"));
                app.recent_kills.lock().unwrap().insert(pid, std::time::Instant::now());
            }
            KillResult::Pending => {
                util::append_log(app, &format!("已送出終止要求 {name} (PID {pid})，等待結束"));
                app.recent_kills.lock().unwrap().insert(pid, std::time::Instant::now());
                killed += 1;
            }
            KillResult::Failed => {
                util::append_log(app, &format!("無法終止 {name} (PID {pid})，所有終止手段皆失敗"));
            }
        }
    }
    killed
}

/// 可中斷等待：等待期間 running 轉 false 或 generation 變更即提前返回（回傳 false）。
/// 以單次 wait_timeout 取代固定 250ms 切片輪詢：閒置期間零週期喚醒，狀態變更時立即喚醒。
fn interruptible_wait(app: &App, gen: u32, dur: Duration) -> bool {
    let deadline = Instant::now() + dur;
    let mut g = app.sleep_lock.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if !app.running.load(Ordering::SeqCst) || app.generation.load(Ordering::SeqCst) != gen {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        g = match app.sleep_cv.wait_timeout(g, deadline - now) {
            Ok((ng, _)) => ng,
            Err(p) => p.into_inner().0,
        };
    }
}

/// 狀態變更（停止／重啟）後呼叫：喚醒所有在 interruptible_wait 中睡眠的執行緒
pub fn wake_guard(app: &App) {
    app.sleep_cv.notify_all();
}

fn run_cycle(app: &App) {
    let (total, avail) = memory_status();
    let used = if total > 0 {
        100.0 - (avail as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    // recover 一律夾在 trigger 以內，維持滯回區間有效
    let (trigger, recover, cooldown, maxkill) = {
        let cfg = app.config.lock().unwrap();
        (
            cfg.trigger_used_percent as f64,
            cfg.recover_used_percent.min(cfg.trigger_used_percent) as f64,
            cfg.cooldown_sec,
            cfg.max_kill_per_cycle.max(1),
        )
    };

    let mut last_action = app.snapshot.lock().unwrap().last_action.clone();

    // 滯回狀態機：>= 觸發門檻進入壓力狀態；要等到 < 恢復門檻才解除，
    // 避免使用率在門檻附近震盪時反覆觸發／解除
    let mut pressured = app.pressured.load(Ordering::SeqCst);
    if !pressured && used >= trigger {
        pressured = true;
        app.pressured.store(true, Ordering::SeqCst);
        util::append_log(
            app,
            &format!("偵測到記憶體壓力（使用率 {:.1}% ≥ {:.0}%），開始介入", used, trigger),
        );
    } else if pressured && used < recover {
        pressured = false;
        app.pressured.store(false, Ordering::SeqCst);
        last_action = format!("壓力解除（使用率 {:.1}% < {:.0}%）", used, recover);
        util::append_log(app, &last_action);
    }

    if pressured {
        let mut lk = app.last_kill.lock().unwrap();
        let elapsed = lk.elapsed().unwrap_or(Duration::from_secs(u64::MAX));
        if elapsed.as_secs() >= cooldown as u64 {
            let killed = kill_top(app, maxkill as usize);
            if killed > 0 {
                *lk = SystemTime::now();
                last_action = format!("已終止 {} 個進程", killed);
                util::append_log(app, &last_action);
            }
        }
    }

    {
        let mut snap = app.snapshot.lock().unwrap();
        snap.running = app.running.load(Ordering::SeqCst);
        snap.pressured = pressured;
        snap.used_percent = used;
        snap.avail_mb = avail;
        snap.total_mb = total;
        // 一併快取觸發門檻，UI 每秒重繪檢查不必再鎖 config
        snap.trigger_percent = trigger as u32;
        snap.last_action = last_action;
    }
}

pub fn spawn_guard(app: Arc<App>, gen: u32) {
    // 護欄執行緒是記憶體壓力最深、最吃堆疊的路徑：run_cycle→kill_top→
    // private_bytes_map/collect_descendants，疊上大量 format! 與 image_base_name
    // 的 2KB 緩衝，再加 Win32/ntdll 內部堆疊。堆疊溢出是不可攔截的 abort
    // （panic=abort 下整程式直接閃退，症狀正是「高負載時消失」）。
    // 故保留 512KB：仍小於預設 1MB，但對最深呼叫鏈有足夠餘裕。
    let worker = app.clone();
    let spawned = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            let app = worker;
            let admin = app.admin;
            let priv_res = enable_debug_privilege();
            match priv_res {
                Ok(()) => util::append_log(&app, "[System] SeDebugPrivilege 已啟用（可終止其他使用者／高權限進程）"),
                Err(1300) => util::append_log(
                    &app,
                    &format!(
                        "[System] SeDebugPrivilege 未獲指派（錯誤碼 1300）：此處理程序不是提權管理員，或系統原則移除了 Debug programs 權限（管理員={admin}）"
                    ),
                ),
                Err(e) => util::append_log(
                    &app,
                    &format!("[System] SeDebugPrivilege 啟用失敗（錯誤碼 {e}；管理員={admin}）"),
                ),
            }
            loop {
                if !app.running.load(Ordering::SeqCst) || app.generation.load(Ordering::SeqCst) != gen {
                    break;
                }
                run_cycle(&app);
                let interval = { app.config.lock().unwrap().interval_sec.max(1) };
                // 可中斷睡眠：停止按鈕按下後立即結束執行緒（notify_all 喚醒），
                // 閒置期間睡滿 interval；壓力期間改用 500ms 快速輪詢，
                // 讓解除偵測與後續終止更即時
                let sleep = if app.pressured.load(Ordering::SeqCst) {
                    Duration::from_millis(500)
                } else {
                    Duration::from_secs(interval as u64)
                };
                interruptible_wait(&app, gen, sleep);
            }
        });
    if spawned.is_err() {
        util::append_log(&app, "[System] 護欄執行緒建立失敗");
    }
}