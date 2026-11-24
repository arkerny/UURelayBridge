//! hosts 往返自检：`UURelayBridge.exe --hosts-selftest <工作目录>`
//!
//! 在临时 hosts 文件上跑一遍 备份 → 写入 → 恢复 → 移除，验证"改了能恢复"。

use std::path::Path;

use super::hosts;

fn check(name: &str, ok: bool, detail: String) -> bool {
    if ok {
        println!("[hosts-selftest] PASS {name} {detail}");
    } else {
        println!("[hosts-selftest] FAIL {name} {detail}");
    }
    ok
}

pub fn run(dir: &str) -> i32 {
    let dir = Path::new(dir);
    if let Err(e) = std::fs::create_dir_all(dir) {
        println!("[hosts-selftest] FAIL mkdir {e}");
        return 1;
    }
    let hosts_file = dir.join("hosts");
    let original = "127.0.0.1 localhost\n10.0.0.1 keep.example\n# user line\n";
    if std::fs::write(&hosts_file, original).is_err() {
        println!("[hosts-selftest] FAIL 无法写临时 hosts");
        return 1;
    }
    std::env::set_var("UURELAYBRIDGE_HOSTS_FILE", &hosts_file);

    let mut ok = true;

    ok &= check("初始未写入", !hosts::is_applied(), String::new());

    let backup = hosts::backup_current(dir);
    ok &= check(
        "备份",
        backup.is_ok(),
        backup.map(|p| p.display().to_string()).unwrap_or_default(),
    );

    let domains = vec!["a.example.com".to_string(), "b.example.com".to_string()];
    let applied = hosts::apply(&domains);
    let text = std::fs::read_to_string(&hosts_file).unwrap_or_default();
    ok &= check(
        "写入",
        applied.map(|n| n == 2).unwrap_or(false)
            && text.contains("127.0.0.1 a.example.com")
            && text.contains("keep.example"),
        format!("{} 字节", text.len()),
    );
    ok &= check("写入后状态", hosts::is_applied(), String::new());

    let restored = hosts::restore_latest(dir);
    let text = std::fs::read_to_string(&hosts_file).unwrap_or_default();
    ok &= check(
        "恢复备份",
        restored.map(|p| p.is_some()).unwrap_or(false) && text == original,
        "内容与原始一致".to_string(),
    );

    // 再写一次然后移除，验证"只删自己那段"
    hosts::apply(&domains).ok();
    let removed = hosts::remove();
    let text = std::fs::read_to_string(&hosts_file).unwrap_or_default();
    ok &= check(
        "移除",
        removed.unwrap_or(false) && !text.contains("a.example.com") && text.contains("keep.example"),
        "用户行保留".to_string(),
    );

    std::env::remove_var("UURELAYBRIDGE_HOSTS_FILE");
    if ok {
        println!("[hosts-selftest] OK");
        0
    } else {
        println!("[hosts-selftest] FAILED");
        1
    }
}
