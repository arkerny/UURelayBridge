//! Hosts 文件管理：只维护自己标记的那一段，其余内容原样保留。
//!
//! 需要管理员权限才能写入系统 hosts；本模块只负责"读写并返回清晰的错误"，
//! 提权由调用方（commands）负责。

use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub const BEGIN: &str = "# >>> UURelayBridge (managed) >>>";
pub const END: &str = "# <<< UURelayBridge (managed) <<<";

/// Hosts 路径。可用 UURELAYBRIDGE_HOSTS_FILE 覆盖（测试 / 便携场景）。
pub fn hosts_path() -> PathBuf {
    if let Ok(p) = std::env::var("UURELAYBRIDGE_HOSTS_FILE") {
        let t = p.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    #[cfg(windows)]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
        PathBuf::from(root).join(r"System32\drivers\etc\hosts")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/etc/hosts")
    }
}

fn read_hosts() -> Result<String, String> {
    let path = hosts_path();
    fs::read_to_string(&path).map_err(|e| {
        format!(
            "读取 hosts 失败（{e}）。路径：{}",
            path.display()
        )
    })
}

/// 去掉我们自己那一段，保留其余内容。
fn strip_block(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim();
        if t == BEGIN {
            inside = true;
            continue;
        }
        if t == END {
            inside = false;
            continue;
        }
        if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

pub fn current_domains() -> Vec<String> {
    let Ok(text) = read_hosts() else {
        return Vec::new();
    };
    let mut inside = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t == BEGIN {
            inside = true;
            continue;
        }
        if t == END {
            inside = false;
            continue;
        }
        if inside {
            if let Some(host) = t.split_whitespace().nth(1) {
                out.push(host.to_string());
            }
        }
    }
    out
}

pub fn is_applied() -> bool {
    let Ok(text) = read_hosts() else {
        return false;
    };
    text.lines().any(|l| l.trim() == BEGIN)
}

/// 写入（或替换）我们的 hosts 段。返回写入的条目数。
pub fn apply(domains: &[String]) -> Result<usize, String> {
    let text = read_hosts()?;
    let base = strip_block(&text);
    let mut out = base;
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out.push_str(BEGIN);
    out.push('\n');
    let mut n = 0;
    for d in domains {
        let d = d.trim();
        if d.is_empty() || d.contains(char::is_whitespace) || d.contains('#') {
            continue;
        }
        out.push_str(&format!("127.0.0.1 {}\n", d));
        n += 1;
    }
    out.push_str(END);
    out.push('\n');

    write_hosts(&out)?;
    Ok(n)
}

pub fn remove() -> Result<bool, String> {
    let text = read_hosts()?;
    if !text.contains(BEGIN) {
        return Ok(false);
    }
    write_hosts(&strip_block(&text))?;
    Ok(true)
}

fn write_hosts(content: &str) -> Result<(), String> {
    let path = hosts_path();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    match fs::OpenOptions::new().write(true).truncate(true).open(&path) {
        Ok(mut f) => f
            .write_all(content.as_bytes())
            .map_err(|e| format!("写入 hosts 失败：{e}")),
        Err(e) => {
            let hint = if e.kind() == std::io::ErrorKind::PermissionDenied {
                "（需要管理员权限）"
            } else {
                ""
            };
            Err(format!(
                "打开 hosts 失败{hint}：{e}。路径：{}",
                path.display()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_tmp_hosts<T>(f: impl FnOnce() -> T) -> T {
        // 这几个用例都靠环境变量指定 hosts 路径，必须串行执行
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        let dir = crate::test_tmp_dir("hosts");
        let file = dir.join("hosts");
        fs::write(&file, "127.0.0.1 localhost\n::1 localhost\n").unwrap();
        std::env::set_var("UURELAYBRIDGE_HOSTS_FILE", &file);
        let out = f();
        std::env::remove_var("UURELAYBRIDGE_HOSTS_FILE");
        out
    }

    #[test]
    fn apply_then_remove_keeps_user_lines() {
        with_tmp_hosts(|| {
            let n = apply(&["a.com".into(), "b.com".into()]).unwrap();
            assert_eq!(n, 2);
            let text = fs::read_to_string(hosts_path()).unwrap();
            assert!(text.contains("127.0.0.1 a.com"));
            assert!(text.contains("127.0.0.1 localhost"));
            assert!(is_applied());

            assert!(remove().unwrap());
            let text = fs::read_to_string(hosts_path()).unwrap();
            assert!(!text.contains("a.com"));
            assert!(text.contains("127.0.0.1 localhost"));
            assert!(!is_applied());
        });
    }

    #[test]
    fn apply_is_idempotent() {
        with_tmp_hosts(|| {
            apply(&["a.com".into()]).unwrap();
            apply(&["a.com".into()]).unwrap();
            let text = fs::read_to_string(hosts_path()).unwrap();
            assert_eq!(text.matches("a.com").count(), 1);
        });
    }

    #[test]
    fn rejects_bad_entries() {
        with_tmp_hosts(|| {
            let n = apply(&["ok.com".into(), "bad domain".into(), "x#y".into()]).unwrap();
            assert_eq!(n, 1);
        });
    }
}
