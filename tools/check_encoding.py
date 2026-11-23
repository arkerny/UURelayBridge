# -*- coding: utf-8 -*-
"""Scan project text files for encoding corruption (replacement chars / PUA)."""
import os

ROOT = r"C:\Users\arkerny\Desktop\hdu-uu-fix\RelayBridge"
EXTS = {".rs", ".js", ".ts", ".html", ".css", ".md", ".json", ".toml", ".yaml", ".yml"}

bad = []
for dirpath, dirnames, filenames in os.walk(ROOT):
    dirnames[:] = [d for d in dirnames if d not in {"target", ".git", "node_modules", ".cachemirror"}]
    for name in filenames:
        if os.path.splitext(name)[1].lower() not in EXTS:
            continue
        p = os.path.join(dirpath, name)
        try:
            raw = open(p, "rb").read()
            s = raw.decode("utf-8")
        except Exception as e:
            bad.append((p, "not-utf8: %s" % e))
            continue
        pua = sum(1 for ch in s if "\ue000" <= ch <= "\uf8ff")
        if "\ufffd" in s or pua:
            bad.append((p, "replacement=%d pua=%d" % (s.count("\ufffd"), pua)))
        if raw.startswith(b"\xef\xbb\xbf"):
            bad.append((p, "has BOM"))

print("files scanned under", ROOT)
if not bad:
    print("OK: no corruption detected")
for p, why in bad:
    print("BAD", os.path.relpath(p, ROOT), why)
