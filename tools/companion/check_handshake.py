"""Cross-checks docs/protocol/fixtures/v1 with Python's hashlib / hmac (COMPANION_PLAN 4.9, 8.1).

Run from the repo root: python -E -P tools/companion/check_handshake.py
Checks handshake.json (transcript, salt, PBKDF2 pair_key, token, both MAC pairs) and the framing of every .hex
fixture (length field = payload length, known type number). Exit code 0 = all good.
"""
import hashlib
import hmac
import json
import pathlib
import struct
import sys

FIX = pathlib.Path(__file__).resolve().parents[2] / "docs" / "protocol" / "fixtures" / "v1"
TYPES = {
    "hello": 0x0001, "auth": 0x0002, "config": 0x0010, "frame_ack": 0x0011, "pen": 0x0012, "touch": 0x0013,
    "key": 0x0014, "text": 0x0015, "mods": 0x0016, "action": 0x0017, "fit_request": 0x0018, "ping": 0x001E,
    "bye_c": 0x001F, "hello_ack": 0x0081, "auth_wait": 0x0082, "auth_ok": 0x0083, "error": 0x008F, "state": 0x0090,
    "frame": 0x0091, "cursor": 0x0092, "pong": 0x009E, "bye_s": 0x009F,
}


def mac(key, *parts):
    return hmac.new(key, b"".join(parts), hashlib.sha256).hexdigest()


def main():
    bad = []
    h = json.loads((FIX / "handshake.json").read_text(encoding="utf-8"))
    b = {k: bytes.fromhex(h[k]) for k in ("desktop_id", "tablet_id", "server_nonce", "client_nonce")}
    t = b["desktop_id"] + b["tablet_id"] + b["server_nonce"] + b["client_nonce"] + struct.pack("<HB", h["proto"], h["link"])
    salt = b"WTEC1-pair" + b["desktop_id"] + b["tablet_id"] + b["server_nonce"] + b["client_nonce"]
    code = h["code_typed"].upper().replace("-", "").replace(" ", "").replace("O", "0").replace("I", "1").replace("L", "1")
    pair_key = hashlib.pbkdf2_hmac("sha256", code.encode(), salt, h["pbkdf2_iters"], 32)
    token = hmac.new(pair_key, b"WTEC1-token", hashlib.sha256).digest()
    want = {
        "code": code,
        "transcript": t.hex(),
        "salt": salt.hex(),
        "pair_key": pair_key.hex(),
        "token": token.hex(),
        "code_mode.client_mac": mac(pair_key, b"WTEC1-client", t),
        "code_mode.server_mac": mac(pair_key, b"WTEC1-server", t),
        "token_mode.client_mac": mac(token, b"WTEC1-client", t),
        "token_mode.server_mac": mac(token, b"WTEC1-server", t),
    }
    for key, value in want.items():
        got = h
        for part in key.split("."):
            got = got[part]
        if got != value:
            bad.append(f"handshake.json {key}: {got} != python {value}")
    for name, ty in TYPES.items():
        data = bytes.fromhex((FIX / f"{name}.hex").read_text(encoding="utf-8").strip())
        length, mtype, flags = struct.unpack_from("<IHH", data)
        if length != len(data) - 8 or mtype != ty or flags != 0:
            bad.append(f"{name}.hex: len {length} (bytes {len(data) - 8}), type 0x{mtype:04x}, flags {flags}")
    for line in bad:
        print("FAIL", line)
    print("handshake.json and", len(TYPES), "fixtures:", "OK" if not bad else f"{len(bad)} problems")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
