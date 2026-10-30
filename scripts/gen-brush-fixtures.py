#!/usr/bin/env python3
"""Generates synthetic brush files used by the panoraminal-io tests.

All fixtures are produced from scratch by this script (no third-party brushes), so
they are free to redistribute. Layouts follow the public descriptions of the
Photoshop ABR format (v1/v2 and v6+ with 8BIM 'samp' / 'desc' sections) and the
GIMP GBR / GIH formats.

    python3 scripts/gen-brush-fixtures.py   # writes crates/panoraminal-io/tests/fixtures/
"""
import math
import os
import struct

OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "panoraminal-io", "tests", "fixtures")


# ---- tip images -----------------------------------------------------------------

def star(w, h):
    """A 5-point star mask, 255 = paint."""
    px = bytearray(w * h)
    cx, cy = w / 2, h / 2
    r = min(w, h) / 2
    for y in range(h):
        for x in range(w):
            dx, dy = (x + 0.5 - cx) / r, (y + 0.5 - cy) / r
            a = math.atan2(dy, dx)
            rr = math.hypot(dx, dy)
            lim = 0.55 + 0.45 * abs(math.cos(2.5 * a))
            px[y * w + x] = 255 if rr <= lim else 0
    return bytes(px)


def ring(w, h):
    px = bytearray(w * h)
    for y in range(h):
        for x in range(w):
            d = math.hypot((x + 0.5 - w / 2) / (w / 2), (y + 0.5 - h / 2) / (h / 2))
            px[y * w + x] = 255 if 0.6 <= d <= 1.0 else 0
    return bytes(px)


def gradient(w, h):
    return bytes(int(255 * x / (w - 1)) for y in range(h) for x in range(w))


def packbits_rows(data, w, h):
    """PackBits-encode each row; returns (row length table, payload)."""
    lens, payload = [], bytearray()
    for y in range(h):
        row = data[y * w:(y + 1) * w]
        enc = bytearray()
        i = 0
        while i < len(row):
            j = i
            while j + 1 < len(row) and row[j + 1] == row[i] and j - i < 127:
                j += 1
            run = j - i + 1
            if run >= 2:
                enc += struct.pack(">bB", -(run - 1), row[i])
                i = j + 1
            else:
                k = i
                while k < len(row) and k - i < 128 and not (k + 1 < len(row) and row[k + 1] == row[k]):
                    k += 1
                k = max(k, i + 1)
                enc += struct.pack(">b", k - i - 1) + bytes(row[i:k])
                i = k
        lens.append(len(enc))
        payload += enc
    return b"".join(struct.pack(">H", n) for n in lens) + bytes(payload)


def sample_block(data, w, h, depth=8, rle=False):
    head = struct.pack(">iiiiHB", 0, 0, h, w, depth, 1 if rle else 0)
    if depth == 16:
        data = b"".join(bytes([v, v]) for v in data)
        return head + data
    return head + (packbits_rows(data, w, h) if rle else data)


def ucs2(s):
    s = s + "\0"
    return struct.pack(">I", len(s)) + s.encode("utf-16-be")


# ---- ABR v1 / v2 ----------------------------------------------------------------

def abr_v12(version, brushes):
    out = struct.pack(">HH", version, len(brushes))
    for b in brushes:
        out += struct.pack(">HI", b[0], len(b[1])) + b[1]
    return out


def computed(spacing, diameter, roundness, angle, hardness):
    return (1, struct.pack(">IHHHhH", 0, spacing, diameter, roundness, angle, hardness))


def sampled(version, spacing, name, data, w, h, rle):
    body = struct.pack(">IH", 0, spacing)
    if version == 2:
        body += ucs2(name)
    body += b"\x01" + struct.pack(">hhhh", 0, 0, h, w)
    body += sample_block(data, w, h, 8, rle)
    return (2, body)


# ---- ABR v6 descriptors ---------------------------------------------------------

def key(k):
    k = k.encode()
    return struct.pack(">I", 0) + k if len(k) == 4 else struct.pack(">I", len(k)) + k


def obj(cls, items):
    out = ucs2("") + key(cls) + struct.pack(">I", len(items))
    for k, v in items:
        out += key(k) + v
    return out


def Objc(cls, items):
    return b"Objc" + obj(cls, items)


def UntF(unit, v):
    return b"UntF" + unit.encode() + struct.pack(">d", v)


def doub(v):
    return b"doub" + struct.pack(">d", v)


def TEXT(s):
    return b"TEXT" + ucs2(s)


def boolean(b):
    return b"bool" + (b"\x01" if b else b"\x00")


def long_(v):
    return b"long" + struct.pack(">i", v)


def enum(t, v):
    return b"enum" + key(t) + key(v)


def VlLs(vals):
    return b"VlLs" + struct.pack(">I", len(vals)) + b"".join(vals)


def brVr(control, jitter, minimum=0.0):
    return Objc("brVr", [("bVTy", long_(control)), ("fStp", long_(25)),
                         ("jitter", UntF("#Prc", jitter)), ("Mnm ", UntF("#Prc", minimum))])


UUID1 = "6a1d3c2e-0000-4000-8000-000000000001"
UUID2 = "6a1d3c2e-0000-4000-8000-000000000002"


def preset_full():
    tip = Objc("sampledBrush", [
        ("Nm  ", TEXT("Star tip")),
        ("Dmtr", UntF("#Pxl", 30.0)),
        ("Angl", UntF("#Ang", 45.0)),
        ("Rndn", UntF("#Prc", 50.0)),
        ("Spcn", UntF("#Prc", 40.0)),
        ("Intr", boolean(True)),
        ("flipX", boolean(True)),
        ("flipY", boolean(False)),
        ("sampledData", TEXT(UUID1)),
    ])
    return Objc("brushPreset", [
        ("Nm  ", TEXT("Estrela ✨ 30")),
        ("Brsh", tip),
        ("useTipDynamics", boolean(True)),
        ("flipX", boolean(False)),
        ("flipY", boolean(True)),
        ("szVr", brVr(2, 30.0, 10.0)),
        ("minimumDiameter", UntF("#Prc", 20.0)),
        ("tiltScale", UntF("#Prc", 200.0)),
        ("angleDynamics", brVr(7, 10.0)),
        ("roundnessDynamics", brVr(0, 25.0)),
        ("minimumRoundness", UntF("#Prc", 40.0)),
        ("useScatter", boolean(True)),
        ("Spcn", boolean(True)),
        ("bothAxes", boolean(True)),
        ("scatterDynamics", brVr(0, 150.0)),
        ("Cnt ", doub(3.0)),
        ("countDynamics", brVr(0, 50.0)),
        ("usePaintDynamics", boolean(True)),
        ("opVr", brVr(2, 20.0)),
        ("prVr", brVr(0, 10.0)),
        ("useColorDynamics", boolean(False)),
        ("useDualBrush", boolean(True)),
        ("dualBrush", Objc("dualBrush", [("useDualBrush", boolean(True)),
                                          ("BlnM", enum("BlnM", "Mltp"))])),
        ("useTexture", boolean(True)),
        ("Txtr", Objc("Ptrn", [("Nm  ", TEXT("x")), ("Idnt", TEXT("y"))])),
        ("Wtdg", boolean(True)),
        ("Nose", boolean(False)),
        ("Rpt ", boolean(False)),
        ("toolOptions", Objc("null", [("brushPreset", boolean(True)),
                                       ("Md  ", enum("BlnM", "Nrml")),
                                       ("Opct", UntF("#Prc", 100.0)),
                                       ("comp", b"comp" + struct.pack(">q", 123456789012)),
                                       ("type", b"type" + ucs2("") + key("Clr ")),
                                       ("tdta", b"tdta" + struct.pack(">I", 3) + b"abc"),
                                       ("UnFl", b"UnFl" + b"#Pxl" + struct.pack(">Id", 1, 7.0))])),
    ])


def preset_computed():
    tip = Objc("computedBrush", [
        ("Dmtr", UntF("#Pxl", 50.0)),
        ("Hrdn", UntF("#Prc", 80.0)),
        ("Angl", UntF("#Ang", -30.0)),
        ("Rndn", UntF("#Prc", 100.0)),
        ("Spcn", UntF("#Prc", 25.0)),
        ("Intr", boolean(True)),
    ])
    return Objc("brushPreset", [("Nm  ", TEXT("Round 50")), ("Brsh", tip)])


def preset_bristle():
    tip = Objc("bristleBrush", [("Dmtr", UntF("#Pxl", 36.0)), ("Spcn", UntF("#Prc", 2.0))])
    return Objc("brushPreset", [("Nm  ", TEXT("Bristle")), ("Brsh", tip)])


def section(tag, body):
    return b"8BIM" + tag.encode() + struct.pack(">I", len(body)) + body


def samp_record(uuid, block, subversion):
    content = bytes([len(uuid)]) + uuid.encode()
    pre = 47 if subversion == 1 else 301
    content += b"\x00" * (pre - len(content))
    content += block
    rec = struct.pack(">I", len(content)) + content
    rec += b"\x00" * ((4 - len(content) % 4) % 4)
    return rec


def abr_v6(version, subversion, samples, presets, extra_sections=()):
    out = struct.pack(">HH", version, subversion)
    samp = b"".join(samp_record(u, blk, subversion) for u, blk in samples)
    out += section("samp", samp)
    for tag, body in extra_sections:
        out += section(tag, body)
    if presets is not None:
        desc = struct.pack(">I", 16) + obj("null", [("Brsh", VlLs(presets))])
        out += section("desc", desc)
    return out


# ---- GIMP -----------------------------------------------------------------------

def gbr(name, w, h, data, bpp=1, spacing=25):
    n = name.encode() + b"\0"
    header = 28 + len(n)
    return struct.pack(">IIIII", header, 2, w, h, bpp) + b"GIMP" + struct.pack(">I", spacing) + n + data


def gih(name, cells):
    out = (name + "\n").encode()
    out += f"{len(cells)} ncells:{len(cells)} cellwidth:16 cellheight:16 step:100 dim:1 cols:1 rows:1 placement:constant rank0:{len(cells)} sel0:random\n".encode()
    for c in cells:
        out += c
    return out


def main():
    os.makedirs(OUT, exist_ok=True)
    s24 = star(24, 24)
    files = {
        "v1.abr": abr_v12(1, [computed(25, 40, 60, 30, 90),
                              sampled(1, 50, "", s24, 24, 24, False)]),
        "v2.abr": abr_v12(2, [sampled(2, 35, "Anel", ring(32, 20), 32, 20, True),
                              sampled(2, 10, "Gradiente", gradient(16, 8), 16, 8, False)]),
        "v6.abr": abr_v6(6, 1,
                         [(UUID1, sample_block(s24, 24, 24, 8, True)),
                          (UUID2, sample_block(ring(20, 20), 20, 20, 8, False))],
                         [preset_full(), preset_computed(), preset_bristle()],
                         extra_sections=[("patt", b"\x00" * 8)]),
        "v10_sub2_16bit.abr": abr_v6(10, 2,
                                     [(UUID1, sample_block(gradient(12, 6), 12, 6, 16, False))],
                                     None),
        "tip.gbr": gbr("Ponto", 8, 8, ring(8, 8), 1, 60),
        "color.gbr": gbr("Cor", 2, 2, bytes([255, 0, 0, 255, 0, 255, 0, 128,
                                             0, 0, 255, 0, 9, 9, 9, 255]), 4, 25),
        "pipe.gih": gih("Pipe", [gbr("a", 16, 16, star(16, 16)), gbr("b", 16, 16, ring(16, 16))]),
    }
    for name, data in files.items():
        with open(os.path.join(OUT, name), "wb") as f:
            f.write(data)
        print(f"{name}: {len(data)} bytes")


if __name__ == "__main__":
    main()
