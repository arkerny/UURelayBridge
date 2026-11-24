//! 历史备份：只备份 RelayBridge 自己的输出文件，只清理自己的旧备份。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 备份文件名前缀，用来保证只清理自己产生的文件。
pub const BACKUP_PREFIX: &str = "relaybridge-";

fn stamp() -> String {
    // 用 epoch 秒 + 毫秒，避免依赖 chrono 的本地时区格式，排序也稳定
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}-{:03}", d.as_secs(), d.subsec_millis())
}

/// 把已存在的脚本复制到 backups/。返回备份路径（原文件不存在则返回 None）。
pub fn backup_file(src: &Path, backups_dir: &Path) -> io::Result<Option<PathBuf>> {
    if !src.exists() {
        return Ok(None);
    }
    fs::create_dir_all(backups_dir)?;
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bak")
        .to_string();
    let dest = backups_dir.join(format!("{}{}.{}", BACKUP_PREFIX, stamp(), ext));
    fs::copy(src, &dest)?;
    Ok(Some(dest))
}

/// 只保留最近 `keep` 份备份（按文件名中的时间戳排序），其余删除。
/// 返回被删除的文件数。
pub fn prune(backups_dir: &Path, keep: usize) -> io::Result<usize> {
    if !backups_dir.exists() {
        return Ok(0);
    }
    let mut items: Vec<PathBuf> = fs::read_dir(backups_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(BACKUP_PREFIX))
                    .unwrap_or(false)
        })
        .collect();
    items.sort();
    if items.len() <= keep {
        return Ok(0);
    }
    let remove_count = items.len() - keep;
    for p in items.into_iter().take(remove_count) {
        let _ = fs::remove_file(p);
    }
    Ok(remove_count)
}

/// 列出备份（从新到旧）。
pub fn list(backups_dir: &Path) -> io::Result<Vec<PathBuf>> {
    if !backups_dir.exists() {
        return Ok(Vec::new());
    }
    let mut items: Vec<PathBuf> = fs::read_dir(backups_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(BACKUP_PREFIX))
                    .unwrap_or(false)
        })
        .collect();
    items.sort();
    items.reverse();
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        crate::test_tmp_dir(tag)
    }

    #[test]
    fn backup_missing_file_is_noop() {
        let d = tmpdir("noop");
        let r = backup_file(&d.join("none.js"), &d.join("backups")).unwrap();
        assert!(r.is_none());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn backup_and_prune_keep_limit() {
        let d = tmpdir("prune");
        let src = d.join("relaybridge.js");
        fs::write(&src, "v1").unwrap();
        let bdir = d.join("backups");
        for i in 0..5 {
            fs::write(&src, format!("v{}", i)).unwrap();
            backup_file(&src, &bdir).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        let before = list(&bdir).unwrap().len();
        assert_eq!(before, 5);
        let removed = prune(&bdir, 2).unwrap();
        assert_eq!(removed, 3);
        assert_eq!(list(&bdir).unwrap().len(), 2);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn prune_ignores_foreign_files() {
        let d = tmpdir("foreign");
        let bdir = d.join("backups");
        fs::create_dir_all(&bdir).unwrap();
        fs::write(bdir.join("user-notes.txt"), "keep me").unwrap();
        fs::write(bdir.join(format!("{}1.js", BACKUP_PREFIX)), "x").unwrap();
        prune(&bdir, 0).unwrap();
        assert!(bdir.join("user-notes.txt").exists());
        fs::remove_dir_all(&d).unwrap();
    }
}
