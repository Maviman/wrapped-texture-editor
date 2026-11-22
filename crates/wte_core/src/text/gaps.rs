// Owner: T1 text geometry (round 4). Keeping gap values with their characters (FEATURE_TEXT 5.12): pure functions on
// the sorted gap list of `TextFx::gaps`. A gap value belongs to the character on its left.
//
// Indices count `char`s of the content, never bytes. Every function keeps the list invariants of `TextFx::gaps`:
// sorted by `after`, at most one entry per character, no zero values.

use crate::text::{GAP_MAX_EM, GAP_MIN_EM, Gap};

/// One gap of a text line as the panel strip shows it.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct GapSlot {
    /// Index of the stored character on the left of the gap.
    pub after: u32,
    pub left: char,
    /// `None`: the end slot of a line whose last character carries a stored value.
    pub right: Option<char>,
    /// The stored value (0.0 without an entry).
    pub em: f32,
}

/// The stored value after character `after`; 0.0 without an entry.
pub fn get(gaps: &[Gap], after: u32) -> f32 {
    match gaps.binary_search_by_key(&after, |g| g.after) {
        Ok(i) => gaps[i].em,
        Err(_) => 0.0,
    }
}

/// Sets the value after character `after` (clamped to `GAP_MIN_EM..=GAP_MAX_EM`); 0.0 removes the entry. A value
/// that is not finite changes nothing.
pub fn set(gaps: &mut Vec<Gap>, after: u32, em: f32) {
    if !em.is_finite() {
        return;
    }
    let em = em.clamp(GAP_MIN_EM, GAP_MAX_EM);
    match gaps.binary_search_by_key(&after, |g| g.after) {
        Ok(i) if em == 0.0 => {
            gaps.remove(i);
        }
        Ok(i) => gaps[i].em = em,
        Err(_) if em == 0.0 => {}
        Err(i) => gaps.insert(i, Gap { after, em }),
    }
}

/// `n` characters were inserted before index `at`: every entry with `after >= at` moves by `n`. An entry with
/// `after == at - 1` stays: text typed into a gap appears to the right of the gap value.
// The scaffold's signature (`&mut Vec`, like the other edit functions) is kept for its callers.
#[allow(clippy::ptr_arg)]
pub fn insert(gaps: &mut Vec<Gap>, at: u32, n: u32) {
    if n == 0 {
        return;
    }
    for g in gaps.iter_mut().filter(|g| g.after >= at) {
        g.after = g.after.saturating_add(n);
    }
}

/// The characters of `range` were removed: their entries go, later entries move down by the length of the range.
pub fn delete(gaps: &mut Vec<Gap>, range: std::ops::Range<u32>) {
    if range.start >= range.end {
        return;
    }
    let len = range.end - range.start;
    gaps.retain(|g| !range.contains(&g.after));
    for g in gaps.iter_mut().filter(|g| g.after >= range.end) {
        g.after -= len;
    }
}

/// One delete and one insert found by comparing `old` and `new` (longest common prefix and suffix, in characters).
/// Used only as a safety net (FEATURE_TEXT 5.12): among equal neighbours it cannot tell where a key was typed.
pub fn shift_for_edit(gaps: &mut Vec<Gap>, old: &str, new: &str) {
    let old: Vec<char> = old.chars().collect();
    let new: Vec<char> = new.chars().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - prefix;
    let suffix = old.iter().rev().zip(new.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
    let removed_end = old.len() - suffix;
    let inserted = new.len() - suffix - prefix;
    delete(gaps, prefix as u32..removed_end as u32);
    insert(gaps, prefix as u32, inserted as u32);
}

/// True for a character that joins the one before it, so no gap exists between them: combining marks (U+0300-036F,
/// U+1AB0-1AFF, U+1DC0-1DFF, U+20D0-20FF, U+FE20-FE2F), variation selectors (U+FE00-FE0F, U+E0100-E01EF), U+200C
/// and U+200D. A fixed list: marks of other scripts are not covered (FEATURE_TEXT 5.12).
pub fn is_joined(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0300..=0x036F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE20..=0xFE2F
            | 0xFE00..=0xFE0F
            | 0xE0100..=0xE01EF
            | 0x200C
            | 0x200D
    )
}

/// True for a stored character the layout skips (a control character other than the tab and the line feed).
fn is_skipped(ch: char) -> bool {
    ch.is_control() && ch != '\t' && ch != '\n'
}

/// Per text line, the gaps that exist, in reading order (FEATURE_TEXT 5.12), with an end slot for a last character
/// that carries a stored value. A pair whose right character `is_joined`, or whose left character is U+200D, has no
/// slot. Skipped control characters are not laid out, so the characters around them are neighbours.
pub fn slots(content: &str, gaps: &[Gap]) -> Vec<Vec<GapSlot>> {
    let mut lines = Vec::new();
    let mut line: Vec<GapSlot> = Vec::new();
    // The last laid-out character of the current line.
    let mut prev: Option<(u32, char)> = None;
    let finish = |line: &mut Vec<GapSlot>, prev: Option<(u32, char)>, lines: &mut Vec<Vec<GapSlot>>| {
        if let Some((after, left)) = prev {
            let em = get(gaps, after);
            if em != 0.0 && left != '\u{200D}' {
                line.push(GapSlot { after, left, right: None, em });
            }
        }
        lines.push(std::mem::take(line));
    };
    for (i, ch) in content.chars().enumerate() {
        let i = i as u32;
        if ch == '\n' {
            finish(&mut line, prev, &mut lines);
            prev = None;
            continue;
        }
        if is_skipped(ch) {
            continue;
        }
        if let Some((after, left)) = prev
            && !is_joined(ch)
            && left != '\u{200D}'
        {
            line.push(GapSlot { after, left, right: Some(ch), em: get(gaps, after) });
        }
        prev = Some((i, ch));
    }
    finish(&mut line, prev, &mut lines);
    lines
}

/// How texts name a character: "E", "(space)", "(tab)"; characters that cannot be seen get their code point.
pub fn slot_name(ch: char) -> String {
    match ch {
        ' ' => "(space)".to_owned(),
        '\t' => "(tab)".to_owned(),
        '\u{A0}' => "(no-break space)".to_owned(),
        c if c.is_control() || c.is_whitespace() || is_joined(c) || matches!(c as u32, 0x200B..=0x200F | 0x2060) => {
            format!("U+{:04X}", c as u32)
        }
        c => c.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(after: u32, em: f32) -> Gap {
        Gap { after, em }
    }

    fn afters(gaps: &[Gap]) -> Vec<u32> {
        gaps.iter().map(|g| g.after).collect()
    }

    #[test]
    fn joined_characters_are_the_fixed_ranges() {
        for ch in ['\u{301}', '\u{36F}', '\u{1AB0}', '\u{1DFF}', '\u{20D0}', '\u{FE20}', '\u{FE0F}', '\u{E0100}', '\u{200C}', '\u{200D}'] {
            assert!(is_joined(ch), "{:x}", ch as u32);
        }
        for ch in ['a', ' ', '\t', '\n', '\u{2FF}', '\u{370}', '\u{200B}', '\u{200E}', '\u{1F600}'] {
            assert!(!is_joined(ch), "{:x}", ch as u32);
        }
    }

    #[test]
    fn get_and_set() {
        let mut gaps = Vec::new();
        set(&mut gaps, 5, 0.25);
        set(&mut gaps, 2, 9.0);
        set(&mut gaps, 7, -3.0);
        assert_eq!(gaps, vec![g(2, GAP_MAX_EM), g(5, 0.25), g(7, GAP_MIN_EM)]);
        assert_eq!(get(&gaps, 5), 0.25);
        assert_eq!(get(&gaps, 4), 0.0);
        set(&mut gaps, 5, 0.0);
        set(&mut gaps, 9, 0.0);
        set(&mut gaps, 2, f32::NAN);
        assert_eq!(gaps, vec![g(2, GAP_MAX_EM), g(7, GAP_MIN_EM)]);
    }

    /// TEXT 8.1 `values_survive_content_edits`.
    #[test]
    fn values_survive_content_edits() {
        // "WTE 2026": gaps after "E" (2) and after "0" (5).
        let base = vec![g(2, 0.3), g(5, -0.2)];
        let mut gaps = base.clone();
        insert(&mut gaps, 0, 1); // "XWTE 2026"
        assert_eq!(afters(&gaps), vec![3, 6]);
        let mut gaps = base.clone();
        insert(&mut gaps, 3, 1); // "WTEX 2026": typed into the first gap
        assert_eq!(afters(&gaps), vec![2, 6]);
        let mut gaps = base.clone();
        delete(&mut gaps, 1..3); // "W 2026"
        assert_eq!(gaps, vec![g(3, -0.2)]);
        // Replace "20" (4..6) by "1999": as delete then insert; the entry of "0" goes with its character.
        let mut gaps = base.clone();
        delete(&mut gaps, 4..6);
        insert(&mut gaps, 4, 4);
        assert_eq!(gaps, vec![g(2, 0.3)]);
        let mut shifted = base.clone();
        shift_for_edit(&mut shifted, "WTE 2026", "WTE 199926");
        assert_eq!(shifted, gaps);
        // Characters, not bytes: "ä€x" with a value after "€" (1); delete "ä".
        let mut gaps = vec![g(1, 0.5)];
        shift_for_edit(&mut gaps, "ä€x", "€x");
        assert_eq!(gaps, vec![g(0, 0.5)]);
        let mut gaps = vec![g(1, 0.5)];
        shift_for_edit(&mut gaps, "ä€x", "äü€x");
        assert_eq!(gaps, vec![g(2, 0.5)]);
    }

    /// TEXT 8.1 `shift_for_edit_keeps_untouched_gaps`.
    #[test]
    fn shift_for_edit_keeps_untouched_gaps() {
        let mut gaps = vec![g(2, 0.3), g(5, -0.2)];
        shift_for_edit(&mut gaps, "WTE 2026", "WTE 2027");
        assert_eq!(gaps, vec![g(2, 0.3), g(5, -0.2)]);
        shift_for_edit(&mut gaps, "WTE 2027", "WTE 2027");
        assert_eq!(gaps, vec![g(2, 0.3), g(5, -0.2)]);
        // Equal neighbours: "aaaa" -> "aaaaa" is read as an insert at the end.
        let mut gaps = vec![g(1, 0.1)];
        shift_for_edit(&mut gaps, "aaaa", "aaaaa");
        assert_eq!(gaps, vec![g(1, 0.1)]);
        // Everything replaced.
        let mut gaps = vec![g(1, 0.1)];
        shift_for_edit(&mut gaps, "ab", "xyz");
        assert!(gaps.is_empty());
    }

    /// TEXT 8.1 `edits_keep_the_invariants`: 10,000 pseudo-random calls from a fixed seed.
    #[test]
    fn edits_keep_the_invariants() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |n: u32| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % u64::from(n.max(1))) as u32
        };
        let mut gaps: Vec<Gap> = Vec::new();
        let mut len = 20u32;
        for _ in 0..10_000 {
            match next(3) {
                0 => {
                    let at = next(len + 1);
                    let n = next(4);
                    insert(&mut gaps, at, n);
                    len += n;
                }
                1 => {
                    let a = next(len + 1);
                    let b = a + next(len - a + 1);
                    delete(&mut gaps, a..b);
                    len -= b - a;
                }
                _ => {
                    let em = (next(11) as f32 - 3.0) * 0.25;
                    set(&mut gaps, next(len.max(1)), em);
                }
            }
            if len < 4 {
                insert(&mut gaps, len, 10);
                len += 10;
            }
            assert!(gaps.windows(2).all(|w| w[0].after < w[1].after), "sorted, one per character: {gaps:?}");
            assert!(gaps.iter().all(|g| g.em != 0.0 && g.em.is_finite()), "no zero: {gaps:?}");
            assert!(gaps.iter().all(|g| (GAP_MIN_EM..=GAP_MAX_EM).contains(&g.em)));
        }
    }

    /// TEXT 8.1 `slots_follow_the_lines`.
    #[test]
    fn slots_follow_the_lines() {
        let lines = slots("AB\nC\tD", &[g(4, 0.5)]);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], vec![GapSlot { after: 0, left: 'A', right: Some('B'), em: 0.0 }]);
        assert_eq!(
            lines[1],
            vec![
                GapSlot { after: 3, left: 'C', right: Some('\t'), em: 0.0 },
                GapSlot { after: 4, left: '\t', right: Some('D'), em: 0.5 },
            ]
        );
        // Empty lines and a skipped control character.
        let lines = slots("A\r\u{1}B\n\nC", &[]);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], vec![GapSlot { after: 0, left: 'A', right: Some('B'), em: 0.0 }]);
        assert!(lines[1].is_empty() && lines[2].is_empty());
        assert_eq!(slots("", &[]), vec![Vec::<GapSlot>::new()]);
    }

    /// TEXT 8.1 `a_stored_end_value_gets_an_end_slot`.
    #[test]
    fn a_stored_end_value_gets_an_end_slot() {
        let lines = slots("AB\nCD", &[g(1, 0.25)]);
        assert_eq!(
            lines[0],
            vec![
                GapSlot { after: 0, left: 'A', right: Some('B'), em: 0.0 },
                GapSlot { after: 1, left: 'B', right: None, em: 0.25 },
            ]
        );
        assert_eq!(lines[1].len(), 1);
        let lines = slots("AB\nCD", &[]);
        assert_eq!(lines[0].len(), 1);
        assert!(lines[0].iter().all(|s| s.right.is_some()));
        // The last line too.
        let lines = slots("AB\nCD", &[g(4, -0.1)]);
        assert_eq!(lines[1].last(), Some(&GapSlot { after: 4, left: 'D', right: None, em: -0.1 }));
    }

    /// TEXT 8.1 `joined_characters_have_no_slot`.
    #[test]
    fn joined_characters_have_no_slot() {
        let lines = slots("ae\u{301}b", &[]);
        let pairs: Vec<(char, Option<char>)> = lines[0].iter().map(|s| (s.left, s.right)).collect();
        assert_eq!(pairs, vec![('a', Some('e')), ('\u{301}', Some('b'))]);
        assert_eq!(lines[0][1].after, 2);
        let lines = slots("x\u{200D}y", &[]);
        assert!(lines[0].is_empty(), "{:?}", lines[0]);
        let lines = slots("ax\u{200D}y", &[]);
        assert_eq!(lines[0].len(), 1);
        assert_eq!((lines[0][0].left, lines[0][0].right), ('a', Some('x')));
    }

    /// TEXT 8.1 `slot_names`.
    #[test]
    fn slot_names() {
        assert_eq!(slot_name('E'), "E");
        assert_eq!(slot_name(' '), "(space)");
        assert_eq!(slot_name('\t'), "(tab)");
        assert_eq!(slot_name('ä'), "ä");
        assert_eq!(slot_name('\u{301}'), "U+0301");
        assert_eq!(slot_name('\u{200D}'), "U+200D");
    }
}
