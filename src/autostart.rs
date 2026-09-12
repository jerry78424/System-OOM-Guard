use std::os::windows::process::CommandExt;
use std::process::Command;

const TASK_NAME: &str = "System-OOM-Guard";
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
}

pub fn disable() {
    let _ = Command::new("schtasks.exe")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}