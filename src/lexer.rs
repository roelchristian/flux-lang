use crate::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    pub start: usize,
    /// Exclusive byte offset.
    pub end: usize,
}

// Match against this set by length, not by its listing order. Unknown binary
// punctuation is split only if each constituent is a recognized token.
const SYMBOLS: &[&str] = &[
    "&&/", "||/", "+//", "?/", "~/", "~:", "!{", "{.", "@@", "<<", ">>", ":=", "::", "->", "&&",
    "||", "?:", "@:", "=:", ">:", "<:", "*=", "*?", "=.", "+:", ":#", "#:", "/:", "\\:", "!>",
    "<@", "@>", "//", "-:", "^/", "<=", ">=", "/=", "==", "~=", "..", "&:", "*:", ".:", "[:", "+/",
    "*/", ">/", "</", "?0", "?1", "?+", "??", "?!", "?=", "#[", "#{", "(", ")", "{", "}", "[", "]",
    ",", ";", ":", "!", "?", "@", "&", "|", "+", "-", "*", "/", "%", "^", "<", ">", "=", "~", "#",
    "\\",
];

fn ident_start(b: u8) -> bool {
    b.is_ascii_lowercase()
}

fn ident_continue(b: u8) -> bool {
    ident_start(b) || b.is_ascii_digit() || b == b'_'
}

fn take_while(bytes: &[u8], mut i: usize, pred: impl Fn(u8) -> bool) -> usize {
    while bytes.get(i).copied().is_some_and(&pred) {
        i += 1;
    }
    i
}

fn escape_end(bytes: &[u8], start: usize) -> Result<usize, Error> {
    let code = bytes
        .get(start + 1)
        .copied()
        .ok_or_else(|| Error::new(start, "unterminated escape"))?;
    match code {
        b'\\' | b'"' | b'\'' | b'n' | b'r' | b't' | b'0' => Ok(start + 2),
        b'u' if bytes.get(start + 2) == Some(&b'{') => {
            let hex_start = start + 3;
            let i = take_while(bytes, hex_start, |b| b.is_ascii_hexdigit());
            if i == hex_start || bytes.get(i) != Some(&b'}') {
                return Err(Error::new(start, "expected Unicode escape \\u{hex}"));
            }
            let mut scalar = 0u32;
            for &digit in &bytes[hex_start..i] {
                scalar = scalar
                    .checked_mul(16)
                    .and_then(|n| n.checked_add((digit as char).to_digit(16).unwrap()))
                    .ok_or_else(|| Error::new(start, "Unicode escape does not encode a scalar"))?;
            }
            if char::from_u32(scalar).is_none() {
                return Err(Error::new(start, "Unicode escape does not encode a scalar"));
            }
            Ok(i + 1)
        }
        _ => Err(Error::new(start, "unknown escape")),
    }
}

fn literal_end(bytes: &[u8], start: usize, quote: u8) -> Result<usize, Error> {
    let mut i = start + 1;
    let mut count = 0;
    while let Some(&b) = bytes.get(i) {
        if b == quote {
            if quote == b'\'' && count != 1 {
                return Err(Error::new(
                    start,
                    "character literal must contain exactly one character",
                ));
            }
            return Ok(i + 1);
        }
        if b.is_ascii_control() {
            return Err(Error::new(i, "unescaped control character in literal"));
        }
        i = if b == b'\\' {
            escape_end(bytes, i)?
        } else {
            i + 1
        };
        count += 1;
    }
    Err(Error::new(start, "unterminated literal"))
}

fn numeral_end(bytes: &[u8], start: usize) -> Result<usize, Error> {
    let digit_start = start + usize::from(bytes[start] == b'-');
    let digit_end = take_while(bytes, digit_start, |b| b.is_ascii_digit());
    let mut i = digit_end;
    let fractional =
        bytes.get(i) == Some(&b'.') && bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
    if fractional {
        i = take_while(bytes, i + 1, |b| b.is_ascii_digit());
    }
    let has_exponent = bytes.get(i) == Some(&b'e');
    if has_exponent {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exponent_start = i;
        i = take_while(bytes, i, |b| b.is_ascii_digit());
        if exponent_start == i || bytes.get(i) != Some(&b'r') {
            return Err(Error::new(
                start,
                "scientific notation requires an exponent and r suffix",
            ));
        }
    }
    if let Some(&suffix) = bytes.get(i) {
        if matches!(suffix, b'n' | b'z' | b'q' | b'd' | b'r' | b'b') {
            if (fractional && matches!(suffix, b'n' | b'z' | b'b'))
                || (suffix == b'b'
                    && (digit_start != start
                        || digit_end != start + 1
                        || !matches!(bytes[start], b'0' | b'1')))
                || (suffix == b'n' && digit_start != start)
            {
                return Err(Error::new(start, "invalid numeric literal for suffix"));
            }
            i += 1;
        }
    }
    if bytes
        .get(i)
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
    {
        return Err(Error::new(i, "expected a token boundary after numeral"));
    }
    Ok(i)
}

pub fn lex(source: &str) -> Result<Vec<Token>, Error> {
    // Enforce ASCII across comments and literals too, and before slicing.
    if let Some(start) = source.bytes().position(|b| !b.is_ascii()) {
        return Err(Error::new(
            start,
            "source must be ASCII; use Unicode escapes in literals",
        ));
    }
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"--") {
            i = take_while(bytes, i + 2, |b| !matches!(b, b'\r' | b'\n'));
            continue;
        }
        let start = i;
        i = match b {
            b'"' => literal_end(bytes, start, b'"')?,
            b'\'' => {
                if bytes.get(i + 1).is_some_and(|b| ident_start(*b)) {
                    let end = take_while(bytes, i + 2, ident_continue);
                    if bytes.get(end) != Some(&b'\'') {
                        end // Apostrophe-prefixed type variable.
                    } else {
                        literal_end(bytes, i, b'\'')?
                    }
                } else {
                    literal_end(bytes, i, b'\'')?
                }
            }
            b'0'..=b'9' => numeral_end(bytes, i)?,
            b'-' if bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => numeral_end(bytes, i)?,
            b'a'..=b'z' => {
                let mut end = take_while(bytes, i + 1, ident_continue);
                if bytes.get(end) == Some(&b'.')
                    && bytes.get(end + 1).is_some_and(|b| ident_start(*b))
                {
                    end = take_while(bytes, end + 2, ident_continue);
                }
                end
            }
            b'_' => {
                if bytes.get(i + 1).is_some_and(|b| ident_continue(*b)) {
                    return Err(Error::new(i, "wildcard _ must stand alone"));
                }
                i + 1
            }
            b'.' if bytes.get(i + 1).is_some_and(|b| ident_start(*b)) => {
                take_while(bytes, i + 2, ident_continue)
            }
            b'.' if bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                take_while(bytes, i + 2, |b| b.is_ascii_digit())
            }
            _ => {
                let op = SYMBOLS
                    .iter()
                    .copied()
                    .filter(|op| bytes[i..].starts_with(op.as_bytes()))
                    .max_by_key(|op| op.len())
                    .ok_or_else(|| Error::new(i, format!("unexpected byte {:?}", b as char)))?;
                if matches!(op, "+//" | "?/" | "~/" | "~:") {
                    return Err(Error::new(i, format!("retired operator {op}")));
                }
                i + op.len()
            }
        };
        tokens.push(Token {
            text: source[start..i].to_owned(),
            start,
            end: i,
        });
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(s: &str) -> Vec<String> {
        lex(s).unwrap().into_iter().map(|t| t.text).collect()
    }

    #[test]
    fn maximal_munch_and_spans() {
        let s = "!{ {. @@ << := &&/ && ?: @: =: >: <: *= *? =. +: # :# #: /: \\: !> :: :n";
        let tokens = lex(s).unwrap();
        assert_eq!(
            parts(s),
            [
                "!{", "{.", "@@", "<<", ":=", "&&/", "&&", "?:", "@:", "=:", ">:", "<:", "*=",
                "*?", "=.", "+:", "#", ":#", "#:", "/:", "\\:", "!>", "::", ":", "n"
            ]
        );
        assert!(tokens.iter().all(|t| t.text == s[t.start..t.end]));
    }

    #[test]
    fn numbers_symbols_and_comments() {
        assert_eq!(
            parts("1..3 -2 - 2 1 -- comment\n// .2 0.2 12.50d 1e-3r 0b ?0"),
            [
                "1", "..", "3", "-2", "-", "2", "1", "//", ".2", "0.2", "12.50d", "1e-3r", "0b",
                "?0"
            ]
        );
        assert!(lex("1e-3").is_err());
        assert!(lex("2b").is_err());
        assert!(lex("+// [1 2]").is_err());
        assert!(lex("Upper").is_err());
        assert!(lex("-- Montréal").is_err());
    }

    #[test]
    fn raw_strings_characters_and_names() {
        let s = r#"'a 'x' '\n' "\u{e9}--" m.f .name _"#;
        assert_eq!(
            parts(s),
            ["'a", "'x'", r"'\n'", r#""\u{e9}--""#, "m.f", ".name", "_"]
        );
        for bad in [r#""\u{d800}""#, r#""\x""#, "'xy'", "_name", "\"no close"] {
            assert!(lex(bad).is_err(), "{bad}");
        }
    }
}
