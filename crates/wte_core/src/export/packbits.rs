// Owner: core-persist (C6). PackBits run-length coding of ARCHITECTURE.md 7.7, used for PSD channel rows.
//
// Stream format: a header byte `n` followed by data. `n` in 0..=127 copies the next `n + 1` bytes literally;
// `n` in 129..=255 repeats the next byte `257 - n` times (2..=128); 128 is a no-op that this encoder never emits.

/// Longest literal block and longest run one header byte can describe.
const MAX_BLOCK: usize = 128;

/// Length (capped at `MAX_BLOCK`) of the run of equal bytes starting at `i`. `i` must be inside `src`.
fn run_len(src: &[u8], i: usize) -> usize {
    let byte = src[i];
    let limit = (src.len() - i).min(MAX_BLOCK);
    let mut n = 1;
    while n < limit && src[i + n] == byte {
        n += 1;
    }
    n
}

/// True when the encoder writes the bytes at `i` as a run: 3 or more equal bytes, or 2 when they end the row.
fn starts_run(src: &[u8], i: usize) -> Option<usize> {
    let run = run_len(src, i);
    if run >= 3 || (run == 2 && i + 2 == src.len()) { Some(run) } else { None }
}

/// PackBits-encodes one row, appending to `out`.
///
/// Emits a run for 3 or more equal bytes (2 at the end of the row) and splits runs and literal blocks at 128 bytes.
/// An empty row appends nothing. The worst case is `len + ceil(len / 128)` bytes.
pub fn encode_row(src: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < src.len() {
        if let Some(run) = starts_run(src, i) {
            out.push((257 - run) as u8);
            out.push(src[i]);
            i += run;
            continue;
        }
        let start = i;
        i += 1;
        while i < src.len() && i - start < MAX_BLOCK && starts_run(src, i).is_none() {
            i += 1;
        }
        out.push((i - start - 1) as u8);
        out.extend_from_slice(&src[start..i]);
    }
}

/// Decodes one PackBits row of `out_len` bytes; None if the data is malformed or of the wrong length.
///
/// The whole of `src` must be consumed and must produce exactly `out_len` bytes. The header byte 128 is skipped,
/// as the format defines.
pub fn decode_row(src: &[u8], out_len: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(out_len);
    let mut i = 0;
    while i < src.len() {
        let header = src[i];
        i += 1;
        match header {
            0..=127 => {
                let n = header as usize + 1;
                let literal = src.get(i..i + n)?;
                if out.len() + n > out_len {
                    return None;
                }
                out.extend_from_slice(literal);
                i += n;
            }
            128 => {}
            129..=255 => {
                let n = 257 - header as usize;
                let byte = *src.get(i)?;
                if out.len() + n > out_len {
                    return None;
                }
                out.resize(out.len() + n, byte);
                i += 1;
            }
        }
    }
    if out.len() == out_len { Some(out) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::XorShift;

    /// Independent decoder written straight from the format description: signed header, no length checks.
    fn naive_decode(src: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut pos = 0usize;
        while pos < src.len() {
            let n = src[pos] as i8 as i32;
            pos += 1;
            if n >= 0 {
                for _ in 0..=n {
                    out.push(src[pos]);
                    pos += 1;
                }
            } else if n != -128 {
                for _ in 0..(1 - n) {
                    out.push(src[pos]);
                }
                pos += 1;
            }
        }
        out
    }

    fn check(src: &[u8]) -> Vec<u8> {
        let mut enc = Vec::new();
        encode_row(src, &mut enc);
        assert_eq!(naive_decode(&enc), src, "naive decoder disagrees for {} bytes", src.len());
        assert_eq!(decode_row(&enc, src.len()).as_deref(), Some(src), "decode_row disagrees for {} bytes", src.len());
        assert!(enc.len() <= src.len() + src.len().div_ceil(128), "{} bytes grew to {}", src.len(), enc.len());
        assert!(!has_noop(&enc), "the encoder emitted the no-op header 128");
        enc
    }

    /// Walks the stream and reports whether a header byte 128 occurs.
    fn has_noop(enc: &[u8]) -> bool {
        let mut pos = 0;
        while pos < enc.len() {
            let n = enc[pos];
            if n == 128 {
                return true;
            }
            pos += if n < 128 { n as usize + 2 } else { 2 };
        }
        false
    }

    const LENGTHS: [usize; 9] = [0, 1, 2, 127, 128, 129, 130, 255, 257];

    #[test]
    fn all_equal_rows() {
        for len in LENGTHS {
            let enc = check(&vec![7u8; len]);
            // Runs of 128, then the rest as one more run (or one literal byte).
            let expected = match len {
                0 => 0,
                n => 2 * n.div_ceil(128),
            };
            assert_eq!(enc.len(), expected, "length {len}");
        }
        assert_eq!(check(&[9, 9]), [255, 9]);
        assert_eq!(check(&[9, 9, 9]), [254, 9]);
        assert_eq!(check(&[5; 128]), [129, 5]);
        assert_eq!(check(&[5; 129]), [129, 5, 0, 5]);
        assert_eq!(check(&[5; 130]), [129, 5, 255, 5]);
    }

    #[test]
    fn alternating_rows() {
        for len in LENGTHS {
            let src: Vec<u8> = (0..len).map(|i| if i % 2 == 0 { 1 } else { 200 }).collect();
            let enc = check(&src);
            assert_eq!(enc.len(), len + len.div_ceil(128), "length {len}");
        }
    }

    #[test]
    fn random_rows() {
        let mut rng = XorShift::new(0x5eed);
        for len in LENGTHS.into_iter().chain([3, 64, 1000, 8192]) {
            // Few distinct values so that runs of every length occur.
            for alphabet in [2u32, 3, 256] {
                let src: Vec<u8> = (0..len).map(|_| (rng.next_u32() % alphabet) as u8).collect();
                check(&src);
            }
            // Long runs separated by noise.
            let mut src = Vec::with_capacity(len);
            while src.len() < len {
                let run = 1 + (rng.next_u32() % 300) as usize;
                let byte = rng.next_u32() as u8;
                for _ in 0..run.min(len - src.len()) {
                    src.push(byte);
                }
            }
            check(&src);
        }
    }

    #[test]
    fn runs_and_literals_mix() {
        // A pair inside the row stays literal, a pair at the end becomes a run.
        assert_eq!(check(&[1, 2, 2, 3]), [3, 1, 2, 2, 3]);
        assert_eq!(check(&[1, 2, 2]), [0, 1, 255, 2]);
        assert_eq!(check(&[1, 2, 3, 3, 3, 4]), [1, 1, 2, 254, 3, 0, 4]);
        // A literal block is split at 128 bytes.
        let literal: Vec<u8> = (0..200u32).map(|i| (i % 251) as u8).collect();
        let enc = check(&literal);
        assert_eq!(enc[0], 127);
        assert_eq!(enc[129], 71);
        assert_eq!(enc.len(), 202);
        // encode_row appends.
        let mut out = vec![42];
        encode_row(&[1, 1, 1], &mut out);
        assert_eq!(out, [42, 254, 1]);
    }

    #[test]
    fn decode_rejects_malformed_input() {
        assert_eq!(decode_row(&[], 0), Some(Vec::new()));
        assert_eq!(decode_row(&[], 1), None);
        // Literal block cut short.
        assert_eq!(decode_row(&[2, 1, 2], 3), None);
        // Run without its byte.
        assert_eq!(decode_row(&[254], 3), None);
        // Too many and too few bytes.
        assert_eq!(decode_row(&[254, 1], 2), None);
        assert_eq!(decode_row(&[254, 1], 4), None);
        assert_eq!(decode_row(&[0, 1, 0, 2], 1), None);
        // The no-op header is skipped.
        assert_eq!(decode_row(&[128, 0, 5, 128], 1), Some(vec![5]));
        assert_eq!(decode_row(&[254, 1], 3), Some(vec![1, 1, 1]));
    }

    #[test]
    fn worst_case_of_an_8192_px_row_fits_u16() {
        let src: Vec<u8> = (0..8192u32).map(|i| (i % 2) as u8).collect();
        let enc = check(&src);
        assert_eq!(enc.len(), 8192 + 64);
        assert!(enc.len() <= u16::MAX as usize);
    }
}
