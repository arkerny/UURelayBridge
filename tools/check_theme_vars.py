# -*- coding: utf-8 -*-
"""一致性检查：深色/浅色主题必须定义同一组 CSS 变量，
且代码里用到的每个 var(--x) 都必须在两个主题里都有定义。"""

import re

CSS = r"C:\Users\arkerny\Desktop\hdu-uu-fix\RelayBridge\ui\style.css"
css = open(CSS, encoding="utf-8").read()


def block(theme):
    m = re.search(r'html\[data-theme="%s"\]\s*\{(.*?)\n\}' % theme, css, re.S)
    return m.group(1) if m else ""


def names(text):
    return set(re.findall(r"--([a-z0-9-]+)\s*:", text))


root = re.search(r":root\s*\{(.*?)\n\}", css, re.S).group(1)
dark, light = block("dark"), block("light")
d, l, r = names(dark), names(light), names(root)

print("defined: root=%d dark=%d light=%d" % (len(r), len(d), len(l)))
missing_in_light = sorted(d - l)
missing_in_dark = sorted(l - d)
print("only in dark :", missing_in_light if missing_in_light else "none")
print("only in light:", missing_in_dark if missing_in_dark else "none")

used = set(re.findall(r"var\(--([a-z0-9-]+)\)", css))
undefined = sorted(used - (d | l | r))
print("used vars  :", len(used))
print("undefined  :", undefined if undefined else "none")

# 主题里定义但从未使用的变量（提示，不报错）
unused = sorted((d | l) - used)
print("unused     :", unused if unused else "none")

ok = not missing_in_light and not missing_in_dark and not undefined
print("\nRESULT:", "OK" if ok else "FAIL")
raise SystemExit(0 if ok else 1)
