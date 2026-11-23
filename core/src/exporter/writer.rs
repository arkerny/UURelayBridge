//! 原子写入：同目录临时文件 → fsync → 替换。失败时保留原文件。

use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// 同一目录下的临时文件名（不用随机数，便于失败后清理）。
fn temp_path(target: &Path) -> io::Result<PathBuf> {
    let dir = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "目标路径没有父目录"))?;
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("relaybridge.out");
    Ok(dir.join(format!(".{}.tmp-{}", name, std::process::id())))
}

/// 原子写入文本。
pub fn atomic_write(path: &Path, contents: &str) -> io::Result<()> {
    atomic_write_bytes(path, contents.as_bytes())
}

pub fn atomic_write_bytes(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = temp_path(path)?;
    let write_result = (|| -> io::Result<()> {
        let mut f = File::create(&tmp)?;
        f.write_all(contents)?;
        f.flush()?;
        // 落盘后再替换，避免断电留下半截文件
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // Windows 上 std::fs::rename 会用 MOVEFILE_REPLACE_EXISTING 覆盖已存在文件
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// 读取文本文件（不存在返回 Ok(None)）。
pub fn read_optional(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("relaybridge-test-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn write_and_overwrite() {
        let d = tmpdir("write");
        let f = d.join("a.txt");
        atomic_write(&f, "one").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "one");
        atomic_write(&f, "two").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "two");
        // 没有留下临时文件
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn creates_missing_dirs() {
        let d = tmpdir("mkdir");
        let f = d.join("nested/deep/b.txt");
        atomic_write(&f, "x").unwrap();
        assert!(f.exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn keeps_old_file_when_write_fails() {
        let d = tmpdir("fail");
        let f = d.join("a.txt");
        atomic_write(&f, "good").unwrap();
        // 目标目录被替换成文件，写入必然失败
        let broken = d.join("b.txt");
        fs::create_dir_all(&broken).unwrap();
        assert!(atomic_write(&broken, "x").is_err());
        assert_eq!(fs::read_to_string(&f).unwrap(), "good");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn unicode_and_space_paths() {
        let d = tmpdir("unicode");
        let f = d.join("中文 目录").join("脚本 空格.js");
        atomic_write(&f, "内容 ok").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "内容 ok");
        fs::remove_dir_all(&d).unwrap();
    }
}
