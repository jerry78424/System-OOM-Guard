use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Condvar, Mutex};
use std::time::{Instant, SystemTime};

use crate::config::Config;
use crate::icons::IconSet;

#[derive(Default)]
pub struct Snapshot {
    pub running: bool,
    /// 滯回狀態機：進入壓力後直到低於恢復門檻才解除
    pub pressured: bool,
    pub used_percent: f64,
    pub avail_mb: u64,
    pub total_mb: u64,
    /// 隨 run_cycle 一併快取，讓 UI 每秒重繪檢查不必再鎖 config
    pub trigger_percent: u32,
    pub last_action: String,
}

pub struct App {
    pub running: AtomicBool,
    pub generation: AtomicU32,
    /// 記憶體壓力滯回狀態：>= 觸發門檻置位，< 恢復門檻清除
    pub pressured: AtomicBool,
    pub config: Mutex<Config>,
    pub snapshot: Mutex<Snapshot>,
    pub log_queue: Mutex<VecDeque<String>>,
    pub last_kill: Mutex<SystemTime>,
    /// 近期已處理的進程（避免 zombie 重複終止），PID → 時間戳
    pub recent_kills: Mutex<HashMap<u32, Instant>>,
    /// 最近一次 Standby 回收時間，用於節流（避免壓力期間每 500ms 都回收一次）
    pub last_standby: Mutex<Instant>,
    pub log_path: String,
    pub config_path: String,
    pub exe_path: String,
    pub admin: bool,
    pub exiting: AtomicBool,
    pub icons: IconSet,
    /// 護欄執行緒可中斷睡眠用的鎖＋條件變數：停止／重啟時 notify_all 立即喚醒
    pub sleep_lock: Mutex<()>,
    pub sleep_cv: Condvar,
}