# System OOM Guard

用 **Rust + 原生 Win32 API** 實作的單檔記憶體護欄（tray 常駐 GUI）。
當系統實體記憶體使用率超過門檻時，自動以「終止整棵子進程樹」的方式終止最吃記憶體的程式（預設鎖定 AI / llama 類工作負載），記憶體壓力解除後停止介入。

編譯出來的單一可執行檔約 430 KB，無外部執行期相依。

## 功能
- **滯回式壓力偵測**：使用率 ≥ 觸發門檻進入壓力；要低於「恢復門檻」才解除，避免門檻附近反覆震盪。
- **終止吃記憶體的大程式**：依「私有記憶體（private commit）」排名選取，每輪最多終止 N 個。
- **樹狀終止（預設）**：趁目標存活先快照整棵子進程樹，連同後代一併終止；可在設定中關閉退回「只殺目標、失敗才升級」。
- **保護名單**：內建最低保障名單（`csrss`、`lsass`、`svchost`、`explorer`… 一律不可殺）＋ 使用者自訂保護名單；對後代進程同樣套用，並做 PID 重用的影像名稱核對。
- **SeDebugPrivilege**：以管理員身分執行時嘗試啟用，可終止其他使用者／高權限進程。
- **單例常駐**：以 `Local\SystemOOMGuardSingleInstance` 互斥鎖確保單實例，重複啟動會把既有視窗帶到前景。
- **托盤圖示 + 狀態列**：即時顯示使用率／可用／門檻與最後動作；支援 explorer 重啟後重建托盤圖示。
- **開機自啟**：寫入登入觸發的排程任務；可選擇程式啟動後是否自動啟用護欄。

## 編譯
需要 Windows + Rust（MSVC toolchain）。

```powershell
cargo build --release
```

產物：`target\release\System-OOM-Guard.exe`

> Release profile 採用 `lto = true`、`codegen-units = 1`、`panic = "abort"`、`strip = true`，以壓細體積。

## 執行 / 命令列參數
- 直接執行：啟動 GUI 常駐；非管理員時會嘗試自我提權（`runas`）。
- `--no-elevate`：跳過自我提權（測試／低權限環境用）。
- `--settings`：啟動時直接開啟設定視窗。
- `--crashtest`：診斷用，故意觸發一次 panic，用來驗證閃退攔截器會寫入 `crash.log`。

建議以**管理員身分**執行，否則終止其他使用者／高權限進程可能失敗。

## 設定選項（config.json，PascalCase）
| 欄位 | 預設 | 說明 |
|---|---|---|
| `TriggerUsedPercent` | 95 | 觸發介入的記憶體使用率門檻（%） |
| `RecoverUsedPercent` | 85 | 低於此值解除壓力（滯回；一律夾在 trigger 內） |
| `IntervalSec` | 5 | 閒置時輪詢間隔（秒）；壓力期間改用 500ms 快速輪詢 |
| `MaxKillPerCycle` | 3 | 每輪最多終止幾個進程 |
| `CooldownSec` | 30 | 兩次終止之间的冷卻（秒） |
| `AiProcessPattern` | `llama\|python\|ollama\|studio` | 無視窗程式的特徵比對（`|` 分隔，小寫主檔名比對） |
| `ProtectedNames` | 見內建 | 使用者保護名單 |
| `AutoStart` | true | 開機自啟（登入排程任務） |
| `AutoGuard` | true | 程式啟動後自動啟用護欄 |
| `KillTree` | true | 終止時連帶殺死整棵子進程樹 |

`config.json` 與 `System-OOM-Guard.log` 會寫在執行檔所在資料夾（已列入 `.gitignore`，不提交）。

## 終止行為重點
- 候選需通過：非自身、不在最低保障名單、不在使用者保護名單、非近期已處理、且（有視窗 或 符合 AI 特徵）。
- `KillTree=true`（預設）：`collect_descendants` 快照後代 → 逐一終止（跳過受保護者、做 PID 重用影像名稱核對）→ 再終止目標。
- 結果型別 `KillResult`：`Killed` / `AlreadyDead` / `Pending`（要求已送出、拆除中，不阻塞）/ `Failed`。
- 近期已處理 PID 有 30 秒冷卻，避免 zombie／拆除中進程被重複選中。

## 閃退偵測與記錄
`release` 採用 `panic = "abort"`，任何 panic／存取違規／堆疊溢出／OOM 都會讓程式瞬間結束且不經正常清理。為此有兩層攔截：
- **Panic 攔截器**：安裝於 `main` 最前段，把 panic 的執行緒、訊息、來源 `檔案:行號` 附加到 `crash.log`。Windows 下存取違規也會經 panic 機制，故多數硬崩潰都能留下來源。（`--crashtest` 可自測此路徑。）
- **執行中 sentinel（`run.pid`）**：啟動寫入目前 PID＋啟動時間；正常結束、或收到系統關機／重啟／登出（`WM_ENDSESSION`）時移除。下次啟動若標記仍殘留，代表上次**未正常結束**（可能堆疊溢出／OOM／被外部終止等不經 hook 的死法），會同時在 `System-OOM-Guard.log` 與 `crash.log` 記錄「上次未正常結束」及前次標記。
- `crash.log`、`run.pid` 寫在執行檔資料夾（已列入 `.gitignore`，不提交）。

## 平台限制
僅支援 **Windows**（大量使用 Win32/ntdll API）。
