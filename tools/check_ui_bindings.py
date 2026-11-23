# -*- coding: utf-8 -*-
"""Static check: every id referenced from app.js exists in index.html."""
import re

HTML = r"C:\Users\arkerny\Desktop\hdu-uu-fix\RelayBridge\ui\index.html"
JS = r"C:\Users\arkerny\Desktop\hdu-uu-fix\RelayBridge\ui\app.js"

html = open(HTML, encoding="utf-8").read()
js = open(JS, encoding="utf-8").read()

ids = set(re.findall(r'id="([^"]+)"', html))
used = set(re.findall(r'\$\("([^"]+)"\)', js))
used |= set(re.findall(r'getElementById\("([^"]+)"\)', js))

missing = sorted(u for u in used if u not in ids)
unused = sorted(i for i in ids if i not in used)

print("html ids   :", len(ids))
print("js refs    :", len(used))
print("MISSING    :", missing if missing else "none")
print("unused ids :", unused if unused else "none")
