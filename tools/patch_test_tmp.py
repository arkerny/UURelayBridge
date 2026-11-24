# -*- coding: utf-8 -*-
"""Point every test temp-dir helper at relaybridge_core::test_tmp_dir."""
import os
import re

ROOT = r"C:\Users\arkerny\Desktop\hdu-uu-fix\RelayBridge\core"

# 1) 把各测试里的 tmpdir 实现替换成统一 helper
targets = [
    os.path.join("src", "exporter", "writer.rs"),
    os.path.join("src", "exporter", "backup.rs"),
    os.path.join("src", "exporter", "mod.rs"),
    os.path.join("src", "settings.rs"),
]

pattern = re.compile(
    r'fn tmpdir\(tag: &str\) -> PathBuf \{\n(?:.*\n)*?    \}\n',
)

replacement = (
    "fn tmpdir(tag: &str) -> PathBuf {\n"
    "        crate::test_tmp_dir(tag)\n"
    "    }\n"
)

for rel in targets:
    p = os.path.join(ROOT, rel)
    s = open(p, encoding="utf-8", newline="").read()
    m = pattern.search(s)
    if not m:
        print("skip (no match):", rel)
        continue
    s = s[: m.start()] + replacement + s[m.end() :]
    open(p, "w", encoding="utf-8", newline="").write(s)
    print("patched", rel)

# 2) manifest.rs 里是内联创建
p = os.path.join(ROOT, "src", "exporter", "manifest.rs")
s = open(p, encoding="utf-8", newline="").read()
old = '''        let d = std::env::temp_dir().join(format!("rb-manifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();'''
new = '''        let d = crate::test_tmp_dir("manifest");
        std::fs::create_dir_all(&d).unwrap();'''
if old in s:
    s = s.replace(old, new)
    open(p, "w", encoding="utf-8", newline="").write(s)
    print("patched src/exporter/manifest.rs")
else:
    print("skip (no match): src/exporter/manifest.rs")
