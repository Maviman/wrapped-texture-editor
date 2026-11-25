"""A minimal Python tablet for the desktop's `companion_host` scenario (desktop-side check, no Android needed).

Usage (repo root, while the exe runs `--scenario companion_host`):
  python -E -P tools/companion/host_probe.py --coords OUT_coords.json [--code K7QF3M9T] [--link usb|wifi]
         [--sessions 1] [--tablet-id HEX32]

Per session: magic, HELLO, AUTH (code mode with --code, else token mode with the token of the previous session, else
confirm mode), then it waits for STATE and the KEY frame, draws one pen stroke with a pressure ramp 0.1 -> 1.0 along
`stroke_row` of the coords file (every sample in order, two per PEN message), waits for a FRAME whose
last_input_seq covers the stroke, acknowledges every frame, and closes with BYE. A second session reconnects with the
token from the first (no code). Prints one line per step; exit 0 = every session confirmed its stroke.
Only ever connects to 127.0.0.1.
"""
import argparse
import hashlib
import hmac
import json
import os
import socket
import struct
import sys
import time

MAGIC = b"WTEC"
T_HELLO, T_AUTH, T_CONFIG, T_ACK, T_PEN, T_PING, T_BYE = 0x0001, 0x0002, 0x0010, 0x0011, 0x0012, 0x001E, 0x001F
S_HELLO_ACK, S_WAIT, S_OK, S_ERROR, S_STATE, S_FRAME = 0x0081, 0x0082, 0x0083, 0x008F, 0x0090, 0x0091
LINKS = {"usb": 1, "wifi": 2}


def pstr(s):
    b = s.encode()
    return struct.pack("<H", len(b)) + b


def frame(ty, payload):
    return struct.pack("<IHH", len(payload), ty, 0) + payload


class Conn:
    def __init__(self, port):
        self.s = socket.create_connection(("127.0.0.1", port), timeout=10)
        self.s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.buf = b""
        self.s.sendall(MAGIC)
        if self.read_exact(4) != MAGIC:
            raise RuntimeError("no WTEC magic")
        self.seq = 0
        self.frames = 0
        self.confirmed = 0
        self.state = None

    def read_exact(self, n):
        while len(self.buf) < n:
            chunk = self.s.recv(65536)
            if not chunk:
                raise RuntimeError("closed by the desktop")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def recv(self, timeout):
        self.s.settimeout(timeout)
        try:
            length, ty, _flags = struct.unpack("<IHH", self.read_exact(8))
            return ty, self.read_exact(length)
        except socket.timeout:
            return None, None

    def send(self, ty, payload):
        self.s.sendall(frame(ty, payload))

    def next_seq(self):
        self.seq += 1
        return self.seq

    def pump(self, seconds):
        """Applies STATE / FRAME (acking frames) for `seconds`."""
        end = time.time() + seconds
        while time.time() < end:
            ty, p = self.recv(max(0.01, end - time.time()))
            if ty is None:
                return
            self.handle(ty, p)

    def handle(self, ty, p):
        if ty == S_STATE:
            w, h, ppp, flags = struct.unpack_from("<HHfI", p)
            self.state = (w, h, ppp, flags)
        elif ty == S_FRAME:
            seq, flags, den, pw, ph, last_input = struct.unpack_from("<IBBHHI", p)
            self.frames += 1
            self.confirmed = max(self.confirmed, last_input)
            if flags & 1:
                self.key = (pw, ph)
            self.send(T_ACK, struct.pack("<IH", seq, 1))
        elif ty == S_ERROR:
            code, n = struct.unpack_from("<HH", p)
            raise RuntimeError(f"ERROR {code}: {p[4:4 + n].decode(errors='replace')}")


def session(port, link, tablet_id, code, token, coords, hold=0.0):
    c = Conn(port)
    nonce = os.urandom(16)
    hello = struct.pack("<HHQB", 1, 1, 0b101, link) + tablet_id + nonce + struct.pack("<HHH", 2960, 1848, 340)
    hello += pstr("host_probe") + pstr("Python probe tablet")
    c.send(T_HELLO, hello)
    ty, p = c.recv(10)
    if ty == S_ERROR:
        c.handle(ty, p)
    if ty != S_HELLO_ACK:
        raise RuntimeError(f"no HELLO_ACK ({ty})")
    proto, _caps = struct.unpack_from("<HQ", p)
    desktop_id, server_nonce = p[10:26], p[26:42]
    modes, iters = struct.unpack_from("<BI", p, 42)
    t = desktop_id + tablet_id + server_nonce + nonce + struct.pack("<HB", proto, link)
    if token is not None and modes & 1:
        mode, key = 1, token
    elif code is not None and modes & 2:
        salt = b"WTEC1-pair" + desktop_id + tablet_id + server_nonce + nonce
        key = hashlib.pbkdf2_hmac("sha256", code.encode(), salt, iters, 32)
        mode = 2
    elif modes & 4:
        mode, key = 3, None
    else:
        raise RuntimeError(f"no usable auth mode (offered {modes:#x})")
    client_mac = hmac.new(key, b"WTEC1-client" + t, hashlib.sha256).digest() if key else bytes(32)
    c.send(T_AUTH, struct.pack("<B", mode) + client_mac)
    while True:
        ty, p = c.recv(70)
        if ty == S_WAIT:
            continue
        if ty == S_ERROR:
            c.handle(ty, p)
        if ty != S_OK:
            raise RuntimeError(f"no AUTH_OK ({ty})")
        break
    server_mac, sent_token = p[1:33], p[33:65]
    if key is not None:
        want = hmac.new(key, b"WTEC1-server" + t, hashlib.sha256).digest()
        if not hmac.compare_digest(want, server_mac):
            raise RuntimeError("the server MAC does not match")
        token = key if mode == 1 else hmac.new(key, b"WTEC1-token", hashlib.sha256).digest()
    else:
        token = sent_token
    print(f"  authenticated (mode {mode})")
    c.send(T_CONFIG, struct.pack("<HHBBH", 2960, 1848, 0, 0, 0))
    deadline = time.time() + 10
    while (c.state is None or c.frames == 0) and time.time() < deadline:
        c.pump(0.1)
    if c.frames == 0:
        raise RuntimeError("no KEY frame")
    print(f"  STATE surface {c.state[0]}x{c.state[1]} ppp {c.state[2]}; KEY frame {c.key[0]}x{c.key[1]}")
    (x0, y0), (x1, y1) = coords["stroke_row"]
    n = 40
    samples = [(0, x0, y0, 0.0)] + [(1 if k == 0 else 2, x0 + (x1 - x0) * k / n, y0 + (y1 - y0) * k / n, 0.1 + 0.9 * k / n)
                                    for k in range(n + 1)] + [(3, x1, y1, 0.0), (4, x1, y1, 0.0)]
    for i in range(0, len(samples), 2):
        chunk = samples[i:i + 2]
        body = b"".join(struct.pack("<BBHffBBH", kind, 0, round(p * 65535), x, y, 255, 0, 8000) for kind, x, y, p in chunk)
        c.send(T_PEN, struct.pack("<IH", c.next_seq(), len(chunk)) + body)
        c.pump(0.016)
    deadline = time.time() + 10
    while c.confirmed < c.seq and time.time() < deadline:
        c.send(T_PING, struct.pack("<I", int(time.time() * 1000) & 0xFFFFFFFF))
        c.pump(0.2)
    ok = c.confirmed >= c.seq
    # --hold: keep the session streaming with pen hover moves at 60 Hz (for frame-time measurements).
    end = time.time() + hold
    k = 0
    while time.time() < end:
        x = x0 + (x1 - x0) * ((k % 120) / 120)
        c.send(T_PEN, struct.pack("<IH", c.next_seq(), 1) + struct.pack("<BBHffBBH", 0, 0, 0, x, y0 - 60, 255, 0, 16000))
        c.pump(0.016)
        k += 1
    if hold > 0:
        c.send(T_PEN, struct.pack("<IH", c.next_seq(), 1) + struct.pack("<BBHffBBH", 4, 0, 0, x0, y0 - 60, 255, 0, 16000))
        c.pump(0.3)
    print(f"  stroke: {c.seq} PEN messages, confirmed {c.confirmed}, {c.frames} frames")
    c.send(T_BYE, b"\x00")
    try:
        c.pump(0.2)
    except RuntimeError:
        pass  # the desktop closes after BYE
    c.s.close()
    return ok, token


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--coords", required=True)
    ap.add_argument("--code")
    ap.add_argument("--link", default="usb", choices=list(LINKS))
    ap.add_argument("--sessions", type=int, default=1)
    ap.add_argument("--tablet-id", default="7072" * 8)
    ap.add_argument("--hold", type=float, default=0.0, help="seconds of hover streaming after each stroke")
    ap.add_argument("--wait", type=float, default=0.0, help="seconds to wait before the first session")
    a = ap.parse_args()
    if a.code:
        # The pairing code as the tablet normalizes it (4.3): upper case, no spaces or dashes, O->0, I->1, L->1.
        a.code = a.code.upper().replace("-", "").replace(" ", "").replace("O", "0").replace("I", "1").replace("L", "1")
    deadline = time.time() + 60
    while not os.path.exists(a.coords) and time.time() < deadline:
        time.sleep(0.2)
    coords = json.load(open(a.coords, encoding="utf-8"))
    time.sleep(a.wait)
    token = None
    good = True
    for k in range(a.sessions):
        print(f"session {k + 1} on 127.0.0.1:{coords['port']}")
        ok, token = session(coords["port"], LINKS[a.link], bytes.fromhex(a.tablet_id), a.code if token is None else None, token, coords, a.hold)
        good &= ok
        time.sleep(0.5)
    print("host_probe:", "OK" if good else "FAILED")
    return 0 if good else 1


if __name__ == "__main__":
    sys.exit(main())
