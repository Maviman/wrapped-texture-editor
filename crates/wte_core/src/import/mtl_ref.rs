// Owner: core-import (C5). Texture references of MTL `map_*` statements (ARCHITECTURE.md 5.21, importer notes of
// section 10): the options in front of the file name are stripped, the file name itself may contain spaces.

/// Number of arguments of a `map_*` option: `(required, optional numeric)`. `None` for anything that is not a
/// known option.
fn option_arity(token: &str) -> Option<(usize, usize)> {
    Some(match token {
        "-blendu" | "-blendv" | "-cc" | "-clamp" | "-imfchan" | "-texres" | "-bm" | "-boost" | "-type" => (1, 0),
        "-mm" => (2, 0),
        "-o" | "-s" | "-t" => (1, 2),
        _ => return None,
    })
}

/// Splits the first whitespace-delimited token off `text` (which must not start with whitespace).
fn split_token(text: &str) -> (&str, &str) {
    match text.find(char::is_whitespace) {
        Some(end) => (&text[..end], text[end..].trim_start()),
        None => (text, ""),
    }
}

/// The file name of a `map_Kd` line: strips leading -options, keeps file names with spaces.
///
/// `map_line` is the rest of the statement after the keyword, e.g. `-s 1 1 1 my tex.png` gives `my tex.png`.
/// Known options: `-blendu`, `-blendv`, `-cc`, `-clamp`, `-imfchan`, `-texres`, `-bm`, `-boost`, `-type` (one
/// argument), `-mm` (two) and `-o`, `-s`, `-t` (one to three numbers). An optional number is only consumed while
/// something is left for the file name, and a token starting with `-` that is not a known option starts the file
/// name. One pair of surrounding double quotes is removed. The result borrows from `map_line` and is empty when
/// the line holds no file name.
pub fn parse_texture_ref(map_line: &str) -> &str {
    let mut rest = map_line.trim();
    loop {
        let (token, after) = split_token(rest);
        let Some((required, optional)) = option_arity(token) else { break };
        let mut next = after;
        for _ in 0..required {
            next = split_token(next).1;
        }
        for _ in 0..optional {
            let (value, tail) = split_token(next);
            if tail.is_empty() || value.parse::<f64>().is_err() {
                break;
            }
            next = tail;
        }
        rest = next;
    }
    if rest.len() >= 2 && rest.starts_with('"') && rest.ends_with('"') {
        rest = rest[1..rest.len() - 1].trim();
    }
    rest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names() {
        assert_eq!(parse_texture_ref("wood.png"), "wood.png");
        assert_eq!(parse_texture_ref("  textures\\wood diffuse.png \r"), "textures\\wood diffuse.png");
        assert_eq!(parse_texture_ref("C:/My Models/tex 01.tga"), "C:/My Models/tex 01.tga");
        assert_eq!(parse_texture_ref(""), "");
        assert_eq!(parse_texture_ref("   "), "");
        assert_eq!(parse_texture_ref("\"quoted name.png\""), "quoted name.png");
    }

    #[test]
    fn scale_option_with_a_name_containing_spaces() {
        // The example of the test plan.
        assert_eq!(parse_texture_ref("-s 1 1 1 my tex.png"), "my tex.png");
        assert_eq!(parse_texture_ref("-s 2 my tex.png"), "my tex.png");
        assert_eq!(parse_texture_ref("-o 0.5 0.25 a.png"), "a.png");
        assert_eq!(parse_texture_ref("-t 1 1 1 -o 0 0 0 -s 4 4 4 brick wall.jpg"), "brick wall.jpg");
    }

    #[test]
    fn fixed_arity_options() {
        assert_eq!(parse_texture_ref("-blendu on -blendv off -clamp on tex.png"), "tex.png");
        assert_eq!(parse_texture_ref("-mm 0 1 -bm 0.5 -boost 2 -texres 512 -cc on -imfchan r tex.png"), "tex.png");
        assert_eq!(parse_texture_ref("-type sphere env.png"), "env.png");
        assert_eq!(parse_texture_ref("-bm\t1.0\tbump map.png"), "bump map.png");
    }

    #[test]
    fn numeric_file_names_survive() {
        // The last token is always left for the file name, even if it looks like a number.
        assert_eq!(parse_texture_ref("-s 1 1 2"), "2");
        assert_eq!(parse_texture_ref("-s 1 1 1 2"), "2");
        assert_eq!(parse_texture_ref("-o 1 001.png"), "001.png");
        // Four numbers after -s: three belong to the option, the rest is the name.
        assert_eq!(parse_texture_ref("-s 1 1 1 1 2.png"), "1 2.png");
    }

    #[test]
    fn unknown_dash_tokens_and_truncated_lines() {
        assert_eq!(parse_texture_ref("-my-texture.png"), "-my-texture.png");
        assert_eq!(parse_texture_ref("-clamp on -weird name.png"), "-weird name.png");
        // Options without a file name give an empty reference instead of panicking.
        assert_eq!(parse_texture_ref("-clamp"), "");
        assert_eq!(parse_texture_ref("-mm 0"), "");
        assert_eq!(parse_texture_ref("-s 1 1 1"), "1");
        assert_eq!(parse_texture_ref("-s"), "");
    }

    #[test]
    fn result_borrows_from_the_input() {
        let line = String::from("-s 1 1 1 caf\u{e9} au lait.png");
        let name = parse_texture_ref(&line);
        assert_eq!(name, "caf\u{e9} au lait.png");
        let offset = name.as_ptr() as usize - line.as_ptr() as usize;
        assert_eq!(&line[offset..], name);
    }
}
