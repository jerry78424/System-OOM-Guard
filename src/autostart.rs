use std::os::windows::process::CommandExt;
use std::process::Command;

const TASK_NAME: &str = "System-OOM-Guard";
const WATCH_TASK: &str = "System-OOM-Guard-Watch";
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn is_enabled() -> bool {
    match Command::new("schtasks.exe")
        .args(["/Query", "/TN", TASK_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

pub fn enable(exe_path: &str) {
    let tr = format!("\"{}\"", exe_path);
    let _ = Command::new("schtasks.exe")
        .args(["/Create", "/TN", TASK_NAME, "/TR", &tr, "/SC", "ONLOGON", "/RL", "HIGHEST", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    fix_limits();
}

pub fn disable() {
    let _ = Command::new("schtasks.exe")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

/// 復活探活工作：每分鐘啟動 `exe --watch`；已在跑則第二實例安靜退出，
/// 已被殺則該次啟動直接成為新實例（≤60 秒復活）。正常結束時由 disable_watch 移除。
pub fn enable_watch(exe_path: &str) {
    let tr = format!("\"{}\" --watch", exe_path);
    let _ = Command::new("schtasks.exe")
        .args(["/Create", "/TN", WATCH_TASK, "/TR", &tr, "/SC", "MINUTE", "/MO", "1", "/RL", "HIGHEST", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    fix_limits();
}

pub fn disable_watch() {
    let _ = Command::new("schtasks.exe")
        .args(["/Delete", "/TN", WATCH_TASK, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

/// schtasks 建立的排程若未明確設定 ExecutionTimeLimit，排程引擎會套用
/// 72 小時預設「逾時即停止工作」——主程式若被 watch 實例收編（探活時無人
/// 在跑、watcher 自我提權後成為主實例），72 小時後就會被排程 TerminateProcess。
/// 明確設 PT0S＝無限期，消除這個自我終止路徑。
pub fn fix_limits() {
    let cmd = "$names='System-OOM-Guard','System-OOM-Guard-Watch'; foreach($n in $names){ $t=Get-ScheduledTask -TaskName $n -EA SilentlyContinue; if($t){ $t.Settings.ExecutionTimeLimit='PT0S'; Set-ScheduledTask -TaskName $n -Settings $t.Settings | Out-Null } }";
    let _ = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", cmd])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}