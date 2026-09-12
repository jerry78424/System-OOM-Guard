use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Config {
    #[serde(default = "default_trigger")] pub trigger_used_percent: u32,
    #[serde(default = "default_recover")] pub recover_used_percent: u32,
    #[serde(default = "default_interval")] pub interval_sec: u32,
    #[serde(default = "default_maxkill")] pub max_kill_per_cycle: u32,
    #[serde(default = "default_cooldown")] pub cooldown_sec: u32,
    #[serde(default = "default_pattern")] pub ai_process_pattern: String,
    #[serde(default = "default_protected")] pub protected_names: Vec<String>,
    #[serde(default = "default_auto")] pub auto_start: bool,
    #[serde(default = "default_auto_guard")] pub auto_guard: bool,
    #[serde(default = "default_kill_tree")] pub kill_tree: bool,
    #[serde(default)] pub window_x: Option<i32>,
    #[serde(default)] pub window_y: Option<i32>,
    #[serde(default)] pub window_width: Option<i32>,
    #[serde(default)] pub window_height: Option<i32>,
}

fn default_trigger() -> u32 { 95 }
fn default_recover() -> u32 { 85 }
fn default_interval() -> u32 { 5 }
fn default_maxkill() -> u32 { 3 }
fn default_cooldown() -> u32 { 30 }
fn default_pattern() -> String { "llama|python|ollama|studio".to_string() }
fn default_auto() -> bool { true }
fn default_auto_guard() -> bool { true }
fn default_kill_tree() -> bool { true }

fn default_protected() -> Vec<String> {
    [
        "System", "Idle", "Registry", "smss", "csrss", "wininit", "winlogon",
        "services", "lsass", "svchost", "spoolsv", "dwm", "explorer",
        "fontdrvhost", "audiodg", "SearchHost", "ShellExperienceHost",
        "StartMenuExperienceHost", "TextInputHost", "RuntimeBroker", "sihost",
        "taskhostw", "ctfmon", "dllhost", "conhost", "OpenConsole",
        "WindowsTerminal", "pwsh", "powershell", "WmiPrvSE", "MsMpEng",
    ].iter().map(|s| s.to_string()).collect()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            trigger_used_percent: default_trigger(),
            recover_used_percent: default_recover(),
            interval_sec: default_interval(),
            max_kill_per_cycle: default_maxkill(),
            cooldown_sec: default_cooldown(),
            ai_process_pattern: default_pattern(),
            protected_names: default_protected(),
            auto_start: default_auto(),
            auto_guard: default_auto_guard(),
            kill_tree: default_kill_tree(),
            window_x: None,
            window_y: None,
            window_width: None,
            window_height: None,
        }
    }
}

impl Config {
    pub fn load(path: &str) -> Config {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(cfg) = serde_json::from_str::<Config>(&text) {
                return cfg;
            }
        }
        Config::default()
    }

    pub fn save(&self, path: &str) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}