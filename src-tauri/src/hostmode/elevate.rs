//! 提权执行 hosts 操作：自己写 hosts 失败时，拉起一个管理员子进程。
//!
//! 子进程就是本程序自己（`--hosts-apply` / `--hosts-remove` / `--hosts-restore`），
//! 只做一次 hosts 读写然后退出，不启动 GUI。

use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::hosts;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpResult {
    pub ok: bool,
    pub message: String,
    pub at: String,
    pub domains: Vec<String>,
}

pub fn op_result_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("hosts-op.json")
}

/// 子进程入口：执行一次 hosts 操作，把结果写到 data_dir/hosts-op.json。
pub fn run_helper(op: &str, data_dir: &str) -> i32 {
    let dir = Path::new(data_dir);
    let cfg_path = dir.join("hostmode.json");
    let domains: Vec<String> = std::fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|t| serde_json::from_str::<super::HostConfig>(&t).ok())
        .map(|c| c.domains)
        .unwrap_or_default();

    let (ok, message) = match op {
        "apply" => match hosts::apply(&domains) {
            Ok(n) => (true, format!("已写入 {n} 条 hosts 记录")),
            Err(e) => (false, e),
        },
        "restore" => match hosts::restore_latest(dir) {
            Ok(Some(p)) => (true, format!("已恢复到备份：{}", p.display())),
            Ok(None) => (false, "没有可用的 hosts 备份".to_string()),
            Err(e) => (false, e),
        },
        _ => match hosts::remove() {
            Ok(true) => (true, "已移除 UURelayBridge 的 hosts 记录".to_string()),
            Ok(false) => (true, "hosts 里没有 UURelayBridge 的记录".to_string()),
            Err(e) => (false, e),
        },
    };

    let res = OpResult {
        ok,
        message,
        at: crate::state::now_rfc3339(),
        domains: hosts::current_domains(),
    };
    let _ = std::fs::write(
        op_result_path(dir),
        serde_json::to_string_pretty(&res).unwrap_or_default(),
    );
    if ok {
        0
    } else {
        1
    }
}

/// 提权执行一次操作（Windows UAC / macOS osascript）。
pub fn elevate_op(op: &str, data_dir: &Path) -> Result<(), String> {
    let verb = match op {
        "apply" => "--hosts-apply",
        "restore" => "--hosts-restore",
        _ => "--hosts-remove",
    };
    run_elevated(verb, data_dir)
}

#[cfg(windows)]
fn run_elevated(verb: &str, data_dir: &Path) -> Result<(), String> {
    use std::ffi::c_void;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            op: *const u16,
            file: *const u16,
            params: *const u16,
            dir: *const u16,
            show: i32,
        ) -> isize;
    }

    let exe = std::env::current_exe().map_err(|e| format!("取自身路径失败：{e}"))?;
    let params = format!("{} \"{}\"", verb, data_dir.display());

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }
    let op = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let prm = wide(&params);

    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            prm.as_ptr(),
            std::ptr::null(),
            1,
        )
    };
    if (r as isize) <= 32 {
        return Err("提权被取消或失败（UAC）".to_string());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_elevated(verb: &str, data_dir: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("取自身路径失败：{e}"))?;
    let cmd = format!("\"{}\" {} \"{}\"", exe.display(), verb, data_dir.display());
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        cmd.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()
        .map_err(|e| format!("调用 osascript 失败：{e}"))?;
    if !status.success() {
        return Err("提权被取消或失败".to_string());
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn run_elevated(_verb: &str, _data_dir: &Path) -> Result<(), String> {
    Err("当前平台不支持自动提权，请用 sudo 运行".to_string())
}

/// 等待提权子进程写回结果。
pub fn wait_result(data_dir: &Path, timeout: Duration) -> Result<OpResult, String> {
    let path = op_result_path(data_dir);
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(r) = serde_json::from_str::<OpResult>(&text) {
                return Ok(r);
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err("等待管理员操作结果超时".to_string())
}
