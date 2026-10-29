#!/usr/bin/env python3
"""Regenerate the bundled CJK font subset from the characters used in every locale.

Requires fonttools (`pip install fonttools`) and Noto Sans CJK Regular (.ttc).
Usage: scripts/subset-cjk-font.py [path/to/NotoSansCJK-Regular.ttc]
"""
import json, pathlib, subprocess, sys

root = pathlib.Path(__file__).resolve().parent.parent
src = sys.argv[1] if len(sys.argv) > 1 else "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"
chars = set("•●↗⌘°×…“”「」『』、。,:;!?()·—–→←↑↓✓✕日本語中文简体")
for f in sorted((root / "crates/panoraminal-i18n/locales").glob("*.json")):
    for v in json.loads(f.read_text(encoding="utf-8")).values():
        chars |= {c for c in v if ord(c) > 0x2000}
txt = root / "target" / "cjk-chars.txt"
txt.parent.mkdir(exist_ok=True)
txt.write_text("".join(sorted(chars)), encoding="utf-8")
out = root / "crates/panoraminal/assets/fonts/NotoSansCJK-Panoraminal-subset.otf"
subprocess.check_call(["pyftsubset", src, "--font-number=0", f"--text-file={txt}",
                       f"--output-file={out}", "--layout-features=*"])
print(f"{len(chars)} glyphs -> {out} ({out.stat().st_size // 1024} KiB)")
