"""Generates complex / awkward test models (OBJ + MTL + PNG, one glTF) for the compatibility sweep.

Usage: python -I make_shapes.py <out_dir>
Every model has UVs unless its name says otherwise; textures are checker PNGs written without third-party modules.
"""
import base64
import json
import math
import os
import random
import struct
import sys
import zlib


def png_bytes(w, h, cells, c0, c1):
    rows = []
    for y in range(h):
        row = bytearray([0])
        for x in range(w):
            c = c0 if ((x * cells // w) + (y * cells // h)) % 2 == 0 else c1
            row += bytes(c)
        rows.append(bytes(row))
    raw = b"".join(rows)

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def write_png(path, w=256, h=256, cells=8, c0=(230, 230, 230), c1=(40, 90, 200)):
    with open(path, "wb") as f:
        f.write(png_bytes(w, h, cells, c0, c1))


class Obj:
    def __init__(self):
        self.v, self.vt, self.vn, self.lines = [], [], [], []

    def vert(self, p):
        self.v.append(p)
        return len(self.v)

    def uv(self, t):
        self.vt.append(t)
        return len(self.vt)

    def face(self, vs, ts=None):
        if ts is None:
            self.lines.append("f " + " ".join(str(a) for a in vs))
        else:
            self.lines.append("f " + " ".join(f"{a}/{b}" for a, b in zip(vs, ts)))

    def write(self, path, mtl=None, materials=None, header=""):
        with open(path, "w", newline="\n") as f:
            f.write(f"# {header}\n")
            if mtl:
                f.write(f"mtllib {mtl}\n")
            for p in self.v:
                f.write("v %.9g %.9g %.9g\n" % p)
            for t in self.vt:
                f.write("vt %.9g %.9g\n" % t)
            for line in self.lines:
                f.write(line + "\n")


def grid_surface(obj, fn, nu, nv, wrap_u, wrap_v, uv_scale=(1.0, 1.0)):
    """Parametric surface fn(u, v) in [0,1]^2 as quads; UV seams duplicated (vt), positions shared when wrapping."""
    idx = {}
    for i in range(nu + 1):
        for j in range(nv + 1):
            ii = i % nu if wrap_u else i
            jj = j % nv if wrap_v else j
            if (ii, jj) not in idx:
                idx[(ii, jj)] = obj.vert(fn(ii / nu, jj / nv))
    tidx = {}
    for i in range(nu + 1):
        for j in range(nv + 1):
            tidx[(i, j)] = obj.uv((i / nu * uv_scale[0], j / nv * uv_scale[1]))
    for i in range(nu):
        for j in range(nv):
            a = (i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)
            vs = [idx[(x % nu if wrap_u else x, y % nv if wrap_v else y)] for x, y in a]
            ts = [tidx[k] for k in a]
            obj.face(vs, ts)


def with_texture(out, name, obj, header, cells=8):
    mtl = name + ".mtl"
    with open(os.path.join(out, mtl), "w", newline="\n") as f:
        f.write(f"newmtl {name}_mat\nKd 1 1 1\nmap_Kd {name}.png\n")
    write_png(os.path.join(out, name + ".png"), cells=cells)
    obj.lines.insert(0, f"usemtl {name}_mat")
    obj.write(os.path.join(out, name + ".obj"), mtl=mtl, header=header)


def torus_knot(out):
    p, q, R, r, tube = 3, 7, 1.0, 0.4, 0.12

    def center(t):
        a = 2 * math.pi * t
        rr = R + r * math.cos(q * a)
        return (rr * math.cos(p * a), rr * math.sin(p * a), r * math.sin(q * a))

    def fn(u, v):
        c = center(u)
        n = center(u + 1e-4)
        tng = [n[k] - c[k] for k in range(3)]
        l = math.sqrt(sum(x * x for x in tng)) or 1
        tng = [x / l for x in tng]
        up = (0, 0, 1)
        b = (tng[1] * up[2] - tng[2] * up[1], tng[2] * up[0] - tng[0] * up[2], tng[0] * up[1] - tng[1] * up[0])
        lb = math.sqrt(sum(x * x for x in b)) or 1
        b = [x / lb for x in b]
        nn = (b[1] * tng[2] - b[2] * tng[1], b[2] * tng[0] - b[0] * tng[2], b[0] * tng[1] - b[1] * tng[0])
        a = 2 * math.pi * v
        return tuple(c[k] + tube * (math.cos(a) * b[k] + math.sin(a) * nn[k]) for k in range(3))

    o = Obj()
    grid_surface(o, fn, 600, 24, True, True, uv_scale=(1.0, 1.0))
    with_texture(out, "torus_knot", o, "torus knot (3,7): one long tube, self-near, 28800 quads", cells=32)


def mobius(out):
    def fn(u, v):
        a = 2 * math.pi * u
        w = (v - 0.5) * 0.8
        return ((1 + w * math.cos(a / 2)) * math.cos(a), (1 + w * math.cos(a / 2)) * math.sin(a), w * math.sin(a / 2))

    o = Obj()
    # Not wrapped in u on purpose for the last column: the strip closes with a twist, so positions of u=1 equal
    # positions of u=0 with v mirrored; weld them by reusing indices.
    nu, nv = 120, 8
    idx = {}
    for i in range(nu):
        for j in range(nv + 1):
            idx[(i, j)] = o.vert(fn(i / nu, j / nv))
    for j in range(nv + 1):
        idx[(nu, j)] = idx[(0, nv - j)]
    t = {}
    for i in range(nu + 1):
        for j in range(nv + 1):
            t[(i, j)] = o.uv((i / nu, j / nv))
    for i in range(nu):
        for j in range(nv):
            a = (i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)
            o.face([idx[k] for k in a], [t[k] for k in a])
    with_texture(out, "mobius", o, "Moebius strip: non-orientable, one boundary, welded twisted seam")


def klein(out):
    def fn(u, v):  # figure-8 immersion
        a, b = 2 * math.pi * u, 2 * math.pi * v
        r = 2.0
        x = (r + math.cos(a / 2) * math.sin(b) - math.sin(a / 2) * math.sin(2 * b)) * math.cos(a)
        y = (r + math.cos(a / 2) * math.sin(b) - math.sin(a / 2) * math.sin(2 * b)) * math.sin(a)
        z = math.sin(a / 2) * math.sin(b) + math.cos(a / 2) * math.sin(2 * b)
        return (x, y, z)

    o = Obj()
    grid_surface(o, fn, 160, 40, False, True)
    with_texture(out, "klein_bottle", o, "Klein bottle figure-8 immersion: self-intersecting, non-orientable")


def nonmanifold(out):
    o = Obj()
    # A "book": 7 pages sharing one spine edge.
    s0, s1 = o.vert((0, 0, 0)), o.vert((0, 1, 0))
    for k in range(7):
        a = 2 * math.pi * k / 7
        p0 = o.vert((math.cos(a), 0, math.sin(a)))
        p1 = o.vert((math.cos(a), 1, math.sin(a)))
        t = [o.uv((k / 7, 0)), o.uv((k / 7, 1)), o.uv(((k + 0.9) / 7, 1)), o.uv(((k + 0.9) / 7, 0))]
        o.face([s0, s1, p1, p0], t)
    # Duplicate face (same vertices twice), flipped duplicate, degenerate (zero-area) triangles, zero-area UVs.
    a, b, c = o.vert((2, 0, 0)), o.vert((3, 0, 0)), o.vert((2, 1, 0))
    ta, tb, tc = o.uv((0.1, 0.1)), o.uv((0.3, 0.1)), o.uv((0.1, 0.3))
    o.face([a, b, c], [ta, tb, tc])
    o.face([a, b, c], [ta, tb, tc])
    o.face([a, c, b], [ta, tc, tb])
    d = o.vert((4, 0, 0))
    o.face([a, b, d], [ta, tb, tc])  # collinear: zero area
    o.face([a, a, b], [ta, ta, tb])  # repeated index
    e, f2, g = o.vert((2, 2, 0)), o.vert((3, 2, 0)), o.vert((2, 3, 0))
    tz = o.uv((0.5, 0.5))
    o.face([e, f2, g], [tz, tz, tz])  # UV-degenerate
    # T-junction: a big quad next to two small quads whose middle vertex lies on the big quad's edge.
    q = [o.vert(p) for p in [(5, 0, 0), (6, 0, 0), (6, 2, 0), (5, 2, 0), (7, 0, 0), (7, 1, 0), (6, 1, 0), (7, 2, 0)]]
    tq = [o.uv((0.6 + i * 0.04, 0.6)) for i in range(4)]
    o.face([q[0], q[1], q[2], q[3]], tq)
    o.face([q[1], q[4], q[5], q[6]], tq)
    o.face([q[6], q[5], q[7], q[2]], tq)
    # A vertex shared by two otherwise disconnected cones (bow-tie vertex).
    apex = o.vert((9, 0, 0))
    for side in (-1, 1):
        ring = [o.vert((9 + side, math.cos(2 * math.pi * k / 12), math.sin(2 * math.pi * k / 12))) for k in range(12)]
        for k in range(12):
            o.face([apex, ring[k], ring[(k + 1) % 12]], [o.uv((0.8, 0.8)), o.uv((0.85 + k * 0.01, 0.9)), o.uv((0.86 + k * 0.01, 0.9))])
    with_texture(out, "nonmanifold", o, "non-manifold book, duplicate / flipped / degenerate faces, T-junction, bow-tie vertex")


def cube_obj(o, c, s, uv_off=(0, 0), uv_s=1.0):
    corners = [(c[0] + s * x, c[1] + s * y, c[2] + s * z) for x in (-0.5, 0.5) for y in (-0.5, 0.5) for z in (-0.5, 0.5)]
    ids = [o.vert(p) for p in corners]
    faces = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)]
    for fi, f in enumerate(faces):
        u0 = uv_off[0] + (fi % 3) / 3 * uv_s
        v0 = uv_off[1] + (fi // 3) / 2 * uv_s
        t = [o.uv((u0, v0)), o.uv((u0 + uv_s / 3, v0)), o.uv((u0 + uv_s / 3, v0 + uv_s / 2)), o.uv((u0, v0 + uv_s / 2))]
        o.face([ids[k] for k in f], t)


def scales(out):
    o = Obj()
    cube_obj(o, (1.0e6, -2.0e6, 5.0e5), 3.0e5)
    with_texture(out, "huge_far_cube", o, "cube of side 3e5 centred at (1e6, -2e6, 5e5): huge scale and far offset")
    o = Obj()
    cube_obj(o, (0.001, 0.0, -0.002), 2.0e-5)
    with_texture(out, "tiny_cube", o, "cube of side 2e-5: tiny scale")
    o = Obj()
    for k in range(40):
        a = 2 * math.pi * k / 40
        p = [o.vert((math.cos(a) * r, math.sin(a) * r, 0.0)) for r in (1.0, 2.0)]
        p += [o.vert((math.cos(a + 2 * math.pi / 40) * r, math.sin(a + 2 * math.pi / 40) * r, 0.0)) for r in (2.0, 1.0)]
        t = [o.uv((k / 40, 0)), o.uv((k / 40, 1)), o.uv(((k + 1) / 40, 1)), o.uv(((k + 1) / 40, 0))]
        o.face(p, t)
    with_texture(out, "flat_annulus", o, "flat ring in z = 0: zero-thickness bounds, hole in the middle (centre ray misses)")


def many_islands(out):
    random.seed(7)
    o = Obj()
    n = 1600
    side = 40
    for k in range(n):
        c = (random.uniform(-10, 10), random.uniform(-10, 10), random.uniform(-10, 10))
        cube_obj(o, c, random.uniform(0.05, 0.6), uv_off=((k % side) / side, (k // side) / side), uv_s=1 / side * 0.95)
    with_texture(out, "many_islands", o, f"{n} disconnected cubes scattered in space, {n * 6} UV islands", cells=64)


def ngons_tiling(out):
    o = Obj()
    lines = []
    # Concave and convex n-gons, negative (relative) indices, UVs outside 0..1, faces without vt mixed in,
    # several materials of which some have no texture file.
    mats = ["tex_a", "tex_b", "missing_tex", "no_map"]
    with open(os.path.join(out, "ngons_tiling.mtl"), "w", newline="\n") as f:
        f.write("newmtl tex_a\nmap_Kd ngons_a.png\n\nnewmtl tex_b\nmap_Kd -s 2 2 1 ngons b.png\n\n")
        f.write("newmtl missing_tex\nmap_Kd does_not_exist.png\n\nnewmtl no_map\nKd 0.8 0.2 0.2\n")
    write_png(os.path.join(out, "ngons_a.png"), cells=6)
    write_png(os.path.join(out, "ngons b.png"), cells=12, c1=(200, 60, 40))
    with open(os.path.join(out, "ngons_tiling.obj"), "w", newline="\n") as f:
        f.write("# n-gons (5..12, concave stars), negative indices, tiling UVs up to 4, faces without vt, 4 materials\n")
        f.write("mtllib ngons_tiling.mtl\n")
        for m in range(4):
            f.write(f"usemtl {mats[m]}\no part{m}\n")
            for k in range(30):
                n = 5 + (k % 8)
                star = k % 2 == 1
                cx, cy, cz = (k % 6) * 3.0, (k // 6) * 3.0, m * 2.0
                for i in range(n):
                    a = 2 * math.pi * i / n
                    r = 1.0 if not star or i % 2 == 0 else 0.45
                    f.write("v %.6f %.6f %.6f\n" % (cx + r * math.cos(a), cy + r * math.sin(a), cz + 0.1 * math.sin(3 * a)))
                if m == 3 and k % 3 == 0:
                    f.write("f " + " ".join(str(-n + i) for i in range(n)) + "\n")  # no vt
                    continue
                for i in range(n):
                    a = 2 * math.pi * i / n
                    f.write("vt %.6f %.6f\n" % (2 + 2 * math.cos(a), 2 + 2 * math.sin(a)))
                f.write("f " + " ".join(f"{-n + i}/{-n + i}" for i in range(n)) + "\n")


def high_poly(out):
    o = Obj()

    def fn(u, v):
        th, ph = 2 * math.pi * u, math.pi * v
        r = 1 + 0.05 * math.sin(20 * th) * math.sin(20 * ph)
        return (r * math.sin(ph) * math.cos(th), r * math.cos(ph), r * math.sin(ph) * math.sin(th))

    grid_surface(o, fn, 1000, 500, True, False)
    with_texture(out, "high_poly_sphere", o, "bumpy UV sphere, 500k quads = 1M triangles, poles collapsed", cells=16)


def double_sided(out):
    o = Obj()

    def sph(r):
        return lambda u, v: (r * math.sin(math.pi * v) * math.cos(2 * math.pi * u), r * math.cos(math.pi * v),
                             r * math.sin(math.pi * v) * math.sin(2 * math.pi * u))

    grid_surface(o, sph(1.0), 64, 32, True, False)
    base = len(o.lines)
    grid_surface(o, sph(1.0), 64, 32, True, False)
    # Flip the winding of the second (coincident) copy.
    for i in range(base, len(o.lines)):
        parts = o.lines[i].split()
        o.lines[i] = "f " + " ".join(reversed(parts[1:]))
    with_texture(out, "coincident_flipped_spheres", o, "two coincident spheres, the second with flipped winding (z-fighting, back faces)")


def gltf_nodes(out):
    # Quad-cube mesh shared by 4 nodes: plain, mirrored (negative scale), rotated + non-uniform scale child of a
    # translated parent, and a node with a matrix. One embedded PNG via data URI, uint32 indices, TEXCOORD_0.
    pos, uvs, idx = [], [], []
    faces = [((0, 0, 1), (1, 0, 0), (0, 1, 0)), ((0, 0, -1), (-1, 0, 0), (0, 1, 0)), ((1, 0, 0), (0, 0, -1), (0, 1, 0)),
             ((-1, 0, 0), (0, 0, 1), (0, 1, 0)), ((0, 1, 0), (1, 0, 0), (0, 0, -1)), ((0, -1, 0), (1, 0, 0), (0, 0, 1))]
    for fi, (n, u, v) in enumerate(faces):
        base = len(pos)
        for a, b in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
            pos.append(tuple(0.5 * (n[k] + a * u[k] + b * v[k]) for k in range(3)))
            uvs.append(((fi % 3 + (a + 1) / 2) / 3, (fi // 3 + (1 - b) / 2) / 2))
        idx += [base, base + 1, base + 2, base, base + 2, base + 3]
    pb = b"".join(struct.pack("<3f", *p) for p in pos)
    ub = b"".join(struct.pack("<2f", *t) for t in uvs)
    ib = b"".join(struct.pack("<I", i) for i in idx)
    buf = pb + ub + ib
    png = png_bytes(128, 128, 8, (250, 250, 250), (220, 40, 40))
    doc = {
        "asset": {"version": "2.0", "generator": "wte compat harness"},
        "scene": 0,
        "scenes": [{"nodes": [0, 1, 2, 4]}],
        "nodes": [
            {"mesh": 0, "translation": [-2, 0, 0]},
            {"mesh": 0, "translation": [0, 0, 0], "scale": [-1, 1, 1]},
            {"translation": [2, 0, 0], "children": [3]},
            {"mesh": 0, "rotation": [0, 0.3826834, 0, 0.9238795], "scale": [0.5, 2, 1]},
            {"mesh": 0, "matrix": [1, 0, 0, 0, 0, 0, -1, 0, 0, 1, 0, 0, 0, 2.5, 0, 1]},
        ],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "TEXCOORD_0": 1}, "indices": 2, "material": 0}]}],
        "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}],
        "textures": [{"source": 0}],
        "images": [{"uri": "data:image/png;base64," + base64.b64encode(png).decode()}],
        "buffers": [{"byteLength": len(buf), "uri": "data:application/octet-stream;base64," + base64.b64encode(buf).decode()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": len(pb)},
            {"buffer": 0, "byteOffset": len(pb), "byteLength": len(ub)},
            {"buffer": 0, "byteOffset": len(pb) + len(ub), "byteLength": len(ib)},
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": len(pos), "type": "VEC3",
             "min": [-0.5, -0.5, -0.5], "max": [0.5, 0.5, 0.5]},
            {"bufferView": 1, "componentType": 5126, "count": len(uvs), "type": "VEC2"},
            {"bufferView": 2, "componentType": 5125, "count": len(idx), "type": "SCALAR"},
        ],
    }
    with open(os.path.join(out, "gltf_node_transforms.gltf"), "w", newline="\n") as f:
        json.dump(doc, f)


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for fn in (torus_knot, mobius, klein, nonmanifold, scales, many_islands, ngons_tiling, double_sided, gltf_nodes, high_poly):
        fn(out)
        print("wrote", fn.__name__)


if __name__ == "__main__":
    main()
