"""Print the structure of a PSD / PSB file: header, image resource 1057, layer records (name, lyid, rect,
channels and their compression, blend key, opacity, clipping, flags, mask, tagged block keys) and the merged
image's compression. Read-only; standard library only. Usage: python -E -P tools/sticker/psd_dump.py FILE...
(STICKER_PLAN P0; used to describe the fixtures in crates/wte_core/tests/fixtures/psd/README.md.)"""
import struct
import sys

LONG_KEYS = {b"LMsk", b"Lr16", b"Lr32", b"Layr", b"Mt16", b"Mt32", b"Mtrn", b"Alph", b"FMsk", b"lnk2",
             b"FEid", b"FXid", b"PxSD"}
COMP = {0: "raw", 1: "rle", 2: "zip", 3: "zip+pred"}


class R:
    def __init__(self, b, psb):
        self.b, self.p, self.psb = b, 0, psb

    def take(self, n):
        if self.p + n > len(self.b):
            raise ValueError(f"truncated at {self.p} (+{n})")
        s = self.b[self.p:self.p + n]
        self.p += n
        return s

    def u8(self): return self.take(1)[0]
    def u16(self): return struct.unpack(">H", self.take(2))[0]
    def i16(self): return struct.unpack(">h", self.take(2))[0]
    def u32(self): return struct.unpack(">I", self.take(4))[0]
    def i32(self): return struct.unpack(">i", self.take(4))[0]
    def u64(self): return struct.unpack(">Q", self.take(8))[0]
    def ln(self): return self.u64() if self.psb else self.u32()


def tagged(r, end, out, depth):
    while r.p + 12 <= end:
        sig = r.take(4)
        if sig not in (b"8BIM", b"8B64"):
            r.p -= 4
            break
        key = r.take(4)
        n = r.u64() if (r.psb and key in LONG_KEYS) else r.u32()
        start = r.p
        info = ""
        if key == b"lyid":
            info = f"={r.u32()}"
        elif key == b"luni":
            cnt = r.u32()
            info = "=" + repr(r.take(2 * cnt).decode("utf-16-be"))
        elif key in (b"lsct", b"lsdk"):
            info = f"=type {r.u32()}"
        elif key == b"iOpa":
            info = f"={r.u8()}"
        out.append(key.decode("latin-1") + info)
        if key in (b"Lr16", b"Lr32", b"Layr"):
            r.p = start
            layer_info(r, start + n, depth, sub=True)
        r.p = start + n
    return out


def layer_info(r, end, depth, sub=False):
    count = r.i16()
    print(f"  layer count {count} ({'first alpha = transparency' if count < 0 else 'plain'}){' [in tagged block]' if sub else ''}")
    recs = []
    for _ in range(abs(count)):
        top, left, bottom, right = r.i32(), r.i32(), r.i32(), r.i32()
        nch = r.u16()
        chans = [(r.i16(), r.ln()) for _ in range(nch)]
        assert r.take(4) == b"8BIM"
        blend = r.take(4).decode("latin-1")
        opacity, clip, flags, _ = r.u8(), r.u8(), r.u8(), r.u8()
        extra = r.u32()
        xend = r.p + extra
        mlen = r.u32()
        mask = None
        if mlen:
            ms = r.p
            mt, ml, mb, mr = r.i32(), r.i32(), r.i32(), r.i32()
            mdef, mflags = r.u8(), r.u8()
            mask = f"mask rect [{ml},{mt},{mr},{mb}] default {mdef} flags {mflags:#x} len {mlen}"
            r.p = ms + mlen
        blen = r.u32()
        ranges = r.take(blen).hex()
        nlen = r.u8()
        name = r.take(nlen).decode("latin-1")
        r.p += (4 - (1 + nlen) % 4) % 4
        keys = tagged(r, xend, [], depth)
        r.p = xend
        recs.append(dict(rect=[left, top, right, bottom], chans=chans, blend=blend, opacity=opacity, clip=clip,
                         flags=flags, mask=mask, ranges=ranges, name=name, keys=keys))
    for rec in recs:
        comps = []
        for cid, clen in rec["chans"]:
            c = struct.unpack(">H", r.b[r.p:r.p + 2])[0] if clen >= 2 else None
            comps.append(f"{cid}:{COMP.get(c, c)}")
            r.p += clen
        rec["comps"] = comps
    for i, rec in enumerate(recs):
        print(f"  [{i}] {rec['name']!r} rect {rec['rect']} blend {rec['blend']} opacity {rec['opacity']} "
              f"clip {rec['clip']} flags {rec['flags']:#x}{' HIDDEN' if rec['flags'] & 2 else ''}")
        print(f"      channels {' '.join(rec['comps'])}; ranges {len(rec['ranges']) // 2} B; tagged {rec['keys']}")
        if rec["mask"]:
            print(f"      {rec['mask']}")
    return recs


def dump(path):
    b = open(path, "rb").read()
    print(f"{path}: {len(b)} bytes")
    if b[:4] != b"8BPS":
        print("  not a PSD")
        return
    ver = struct.unpack(">H", b[4:6])[0]
    r = R(b, ver == 2)
    r.p = 12
    ch, h, w, depth, mode = r.u16(), r.u32(), r.u32(), r.u16(), r.u16()
    print(f"  version {ver} ({'PSB' if ver == 2 else 'PSD'}) {w}x{h} channels {ch} depth {depth} mode {mode}")
    n = r.u32()  # colour mode data
    r.p += n
    res_end = r.u32() + r.p
    ids = []
    while r.p < res_end:
        assert r.take(4) == b"8BIM"
        rid = r.u16()
        nl = r.u8()
        r.p += nl + ((1 + nl) % 2)  # Pascal name padded to an even length
        size = r.u32()
        data = r.take(size)
        if size % 2:
            r.p += 1
        ids.append(rid)
        if rid == 1057:
            print(f"  resource 1057 (version info): hasRealMergedData = {data[4]}")
    print(f"  image resources {ids}")
    lm_len = r.ln()
    lm_end = r.p + lm_len
    if lm_len:
        li_len = r.ln()
        li_end = r.p + li_len
        if li_len:
            layer_info(r, li_end, depth)
        r.p = li_end
        gm = r.u32()  # global layer mask info
        r.p += gm
        keys = tagged(r, lm_end, [], depth)
        print(f"  global tagged blocks {keys}")
    r.p = lm_end
    comp = r.u16()
    print(f"  merged image: compression {COMP.get(comp, comp)}, {ch} channels")


if __name__ == "__main__":
    for p in sys.argv[1:]:
        dump(p)
        print()
