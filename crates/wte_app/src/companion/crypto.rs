//! Local crypto for the companion handshake (COMPANION_PLAN C8, protocol 4.3): SHA-256 (FIPS 180-4), HMAC-SHA256
//! (RFC 2104), PBKDF2-HMAC-SHA256 (RFC 8018), a constant-time compare, hex, the Crockford pairing code and
//! `random_bytes` (the OS generator: `BCryptGenRandom` through a `raw-dylib` import on Windows). No crypto crate is in
//! `Cargo.lock`; every test vector below was generated with Python's `hashlib` / `hmac` (`docs/notes/companion/d1.md`).

/// SHA-256 round constants.
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
    0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
    0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 initial hash value.
const H0: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];

/// Streaming SHA-256.
#[derive(Clone)]
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    fill: usize,
    total: u64,
}

impl Default for Sha256 {
    fn default() -> Sha256 {
        Sha256::new()
    }
}

impl Sha256 {
    pub fn new() -> Sha256 {
        Sha256 { state: H0, block: [0; 64], fill: 0, total: 0 }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.fill > 0 {
            let take = (64 - self.fill).min(data.len());
            self.block[self.fill..self.fill + take].copy_from_slice(&data[..take]);
            self.fill += take;
            data = &data[take..];
            if self.fill < 64 {
                return;
            }
            let block = self.block;
            compress(&mut self.state, &block);
            self.fill = 0;
        }
        let (blocks, rest) = data.as_chunks::<64>();
        for block in blocks {
            compress(&mut self.state, block);
        }
        self.block[..rest.len()].copy_from_slice(rest);
        self.fill = rest.len();
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let bits = self.total.wrapping_mul(8);
        self.update(&[0x80]);
        while self.fill != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        debug_assert_eq!(self.fill, 0);
        let mut out = [0u8; 32];
        for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            *chunk = word.to_be_bytes();
        }
        out
    }
}

/// One SHA-256 compression.
fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
        w[i] = u32::from_be_bytes(*word);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ (!e & g);
        let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *s = s.wrapping_add(v);
    }
}

/// SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize()
}

/// HMAC-SHA256 with its key already absorbed (the inner and outer states after the padded key block).
#[derive(Clone)]
pub struct HmacKey {
    inner: Sha256,
    outer: Sha256,
}

impl HmacKey {
    pub fn new(key: &[u8]) -> HmacKey {
        let mut block = [0u8; 64];
        if key.len() > 64 {
            block[..32].copy_from_slice(&sha256(key));
        } else {
            block[..key.len()].copy_from_slice(key);
        }
        let mut inner = Sha256::new();
        let mut outer = Sha256::new();
        inner.update(&block.map(|b| b ^ 0x36));
        outer.update(&block.map(|b| b ^ 0x5c));
        HmacKey { inner, outer }
    }

    /// The MAC of the concatenation of `parts`.
    pub fn mac(&self, parts: &[&[u8]]) -> [u8; 32] {
        let mut inner = self.inner.clone();
        for p in parts {
            inner.update(p);
        }
        let mut outer = self.outer.clone();
        outer.update(&inner.finalize());
        outer.finalize()
    }
}

/// HMAC-SHA256 of the concatenation of `parts` under `key`.
pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    HmacKey::new(key).mac(parts)
}

/// PBKDF2-HMAC-SHA256 (RFC 8018 5.2) into `out` (any length).
pub fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32, out: &mut [u8]) {
    let key = HmacKey::new(password);
    for (i, chunk) in out.chunks_mut(32).enumerate() {
        let index = (i as u32 + 1).to_be_bytes();
        let mut u = key.mac(&[salt, &index]);
        let mut t = u;
        for _ in 1..iterations.max(1) {
            u = key.mac(&[&u]);
            for (a, b) in t.iter_mut().zip(u) {
                *a ^= b;
            }
        }
        chunk.copy_from_slice(&t[..chunk.len()]);
    }
}

/// Equality of two byte strings in time that depends only on their lengths.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    // `black_box` keeps the optimizer from turning the loop into an early exit.
    std::hint::black_box(diff) == 0
}

/// Lower-case hex.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 15) as usize] as char);
    }
    s
}

/// Hex (either case, no separators) to bytes; `None` for odd length or a non-hex character.
pub fn unhex(text: &str) -> Option<Vec<u8>> {
    let t = text.as_bytes();
    if !t.len().is_multiple_of(2) {
        return None;
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    t.chunks(2).map(|p| Some(nibble(p[0])? << 4 | nibble(p[1])?)).collect()
}

/// A fixed-size array from hex.
pub fn unhex_array<const N: usize>(text: &str) -> Option<[u8; N]> {
    unhex(text)?.try_into().ok()
}

/// The pairing code alphabet (Crockford base32: no I, L, O, U).
pub const CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Characters of a pairing code.
pub const CODE_LEN: usize = 8;

/// A pairing code from 40 random bits (5 bytes): 8 characters of `CODE_ALPHABET`.
pub fn code_from_bytes(bytes: [u8; 5]) -> String {
    let mut v = 0u64;
    for b in bytes {
        v = v << 8 | u64::from(b);
    }
    (0..CODE_LEN).rev().map(|i| CODE_ALPHABET[((v >> (i * 5)) & 31) as usize] as char).collect()
}

/// A new random pairing code.
pub fn new_code() -> String {
    let mut b = [0u8; 5];
    random_bytes(&mut b);
    code_from_bytes(b)
}

/// The canonical form of a typed code (protocol 4.3): upper case, spaces and '-' removed, O -> 0, I and L -> 1; `None`
/// unless that leaves exactly 8 characters of the alphabet.
pub fn normalize_code(input: &str) -> Option<String> {
    let mut out = String::with_capacity(CODE_LEN);
    for c in input.chars() {
        let c = match c.to_ascii_uppercase() {
            ' ' | '-' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !c.is_ascii() || !CODE_ALPHABET.contains(&(c as u8)) {
            return None;
        }
        out.push(c);
    }
    (out.len() == CODE_LEN).then_some(out)
}

/// The code as shown in the dialog: "K7QF-3M9T".
pub fn display_code(code: &str) -> String {
    if code.len() == CODE_LEN { format!("{}-{}", &code[..4], &code[4..]) } else { code.to_owned() }
}

/// Fills `buf` from the operating system's random generator. Panics only if the OS generator fails, which on Windows
/// 10 and later it does not.
pub fn random_bytes(buf: &mut [u8]) {
    #[cfg(windows)]
    {
        #[link(name = "bcrypt", kind = "raw-dylib")]
        unsafe extern "system" {
            fn BCryptGenRandom(algorithm: *mut std::ffi::c_void, buffer: *mut u8, len: u32, flags: u32) -> i32;
        }
        /// `BCRYPT_USE_SYSTEM_PREFERRED_RNG`.
        const SYSTEM_RNG: u32 = 2;
        for chunk in buf.chunks_mut(1 << 20) {
            // SAFETY: the buffer is valid for `chunk.len()` bytes; a null algorithm handle is allowed with
            // BCRYPT_USE_SYSTEM_PREFERRED_RNG.
            let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), chunk.as_mut_ptr(), chunk.len() as u32, SYSTEM_RNG) };
            assert!(status >= 0, "BCryptGenRandom failed: 0x{status:08x}");
        }
    }
    #[cfg(not(windows))]
    {
        use std::io::Read;
        std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(buf)).expect("/dev/urandom");
    }
}

/// `N` random bytes.
pub fn random_array<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    random_bytes(&mut b);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Vec<u8> {
        unhex(s).unwrap()
    }

    /// FIPS 180-4 examples; expected values from Python `hashlib.sha256`.
    #[test]
    fn sha256_vectors() {
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // A million "a", fed in uneven pieces through the streaming interface.
        let mut s = Sha256::new();
        let a = vec![b'a'; 1_000_000];
        let mut at = 0;
        for (i, step) in [1usize, 63, 64, 65, 127, 1000].iter().cycle().enumerate() {
            if at >= a.len() {
                break;
            }
            let end = (at + step + i % 3).min(a.len());
            s.update(&a[at..end]);
            at = end;
        }
        assert_eq!(hex(&s.finalize()), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
        assert_eq!(sha256(&a), sha256(&a[..]), "one call == streaming");
    }

    /// RFC 4231 cases 1, 2, 3 and 6; expected values from Python `hmac`.
    #[test]
    fn hmac_vectors() {
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], &[b"Hi There"])),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", &[b"what do ya ", b"want for nothing?"])),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(hex(&hmac_sha256(&[0xaa; 20], &[&[0xdd; 50]])), "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe");
        assert_eq!(
            hex(&hmac_sha256(&[0xaa; 131], &[b"Test Using Larger Than Block-Size Key - Hash Key First"])),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    /// PBKDF2-HMAC-SHA256 "password" / "salt"; expected values from Python `hashlib.pbkdf2_hmac`.
    #[test]
    fn pbkdf2_vectors() {
        let run = |p: &[u8], s: &[u8], n: u32, len: usize| {
            let mut out = vec![0u8; len];
            pbkdf2_hmac_sha256(p, s, n, &mut out);
            hex(&out)
        };
        assert_eq!(run(b"password", b"salt", 1, 32), "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b");
        assert_eq!(run(b"password", b"salt", 2, 32), "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43");
        assert_eq!(run(b"password", b"salt", 4096, 32), "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a");
        assert_eq!(
            run(b"passwordPASSWORDpassword", b"saltSALTsaltSALTsaltSALTsaltSALTsalt", 4096, 40),
            "348c89dbcbd32b2f32d814b8116e84cf2b17347ebc1800181c4e2a1fb8dd53e1c635518c7dac47e9"
        );
    }

    #[test]
    fn constant_time_compare_and_hex() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
        assert!(ct_eq(b"", b""));
        assert_eq!(hex(&[0x00, 0x9f, 0xff]), "009fff");
        assert_eq!(h("009FfF"), vec![0x00, 0x9f, 0xff]);
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
        assert_eq!(unhex_array::<2>("0102"), Some([1, 2]));
        assert_eq!(unhex_array::<3>("0102"), None);
    }

    #[test]
    fn pairing_codes() {
        assert_eq!(code_from_bytes([0; 5]), "00000000");
        assert_eq!(code_from_bytes([0xff; 5]), "ZZZZZZZZ");
        // 40 bits, 5 per character, most significant first.
        assert_eq!(code_from_bytes([0x08, 0x42, 0x10, 0x84, 0x21]), "11111111");
        for _ in 0..50 {
            let c = new_code();
            assert_eq!(c.len(), 8);
            assert!(c.bytes().all(|b| CODE_ALPHABET.contains(&b)), "{c}");
            assert_eq!(normalize_code(&c).as_deref(), Some(c.as_str()));
        }
        assert_eq!(normalize_code("k7qf-3m9t").as_deref(), Some("K7QF3M9T"));
        assert_eq!(normalize_code(" K7QF 3M9T ").as_deref(), Some("K7QF3M9T"));
        assert_eq!(normalize_code("o1il-0000").as_deref(), Some("01110000"));
        assert_eq!(normalize_code("K7QF3M9"), None, "seven characters");
        assert_eq!(normalize_code("K7QF3M9TX"), None, "nine characters");
        assert_eq!(normalize_code("K7QF3M9U"), None, "U is not in the alphabet");
        assert_eq!(normalize_code("K7QF3M9Ä"), None);
        assert_eq!(display_code("K7QF3M9T"), "K7QF-3M9T");
    }

    #[test]
    fn random_bytes_are_not_constant() {
        let a: [u8; 32] = random_array();
        let b: [u8; 32] = random_array();
        assert_ne!(a, b);
        assert_ne!(a, [0; 32]);
    }
}
