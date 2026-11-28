//! Practical, bounded standard-library operations for the typed evaluator.
use super::{operations041, *};
use crate::errors041::{IoFault, IoKind};
use regex::RegexBuilder;
use std::{
    io::{BufRead, Read},
    path::Path,
};

const LINE_LIMIT: usize = 1024 * 1024;
const TEXT_LIMIT: usize = 8 * 1024 * 1024;
const REGEX_PATTERN_LIMIT: usize = 64 * 1024;
const REGEX_SIZE_LIMIT: usize = 1024 * 1024;
const DIRECTORY_NAMES_LIMIT: usize = 8 * 1024 * 1024;

fn string(s: impl AsRef<str>) -> V {
    V::String(Rc::from(s.as_ref()))
}

fn optional(v: Option<V>) -> V {
    V::Optional(v.map(Rc::new))
}

fn sequence(items: Vec<V>) -> V {
    V::Sequence(items.into())
}

fn read_line(input: &mut dyn BufRead) -> Result<V, IoFault> {
    const SOURCE: &str = "stdin";
    let mut raw = Vec::new();
    // One extra byte for CRLF and one for the delimiter/detection. On
    // oversize input each call consumes at most this bounded prefix.
    (&mut *input)
        .take((LINE_LIMIT + 2) as u64)
        .read_until(b'\n', &mut raw)
        .map_err(|e| IoFault::system(SOURCE, e, false))?;
    if raw.is_empty() {
        return Ok(optional(None));
    }
    if raw.last() == Some(&b'\n') {
        raw.pop();
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
    }
    if raw.len() > LINE_LIMIT {
        return Err(IoFault::details(
            IoKind::Read,
            SOURCE,
            "input line exceeds the 1 MiB byte limit".into(),
        ));
    }
    let line = String::from_utf8(raw).map_err(|_| IoFault::encoding(SOURCE))?;
    Ok(optional(Some(string(line))))
}

fn directory(path: &str) -> Result<V, IoFault> {
    let entries = std::fs::read_dir(path).map_err(|e| IoFault::system(path, e, false))?;
    let mut names = Vec::new();
    let mut bytes = 0usize;
    for item in entries {
        let item = item.map_err(|e| IoFault::system(path, e, false))?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| IoFault::encoding(path))?;
        bytes = bytes.saturating_add(name.len());
        if names.len() >= COLLECTION_LIMIT || bytes > DIRECTORY_NAMES_LIMIT {
            return Err(IoFault::details(
                IoKind::Read,
                path,
                "directory listing exceeds the resource limit".into(),
            ));
        }
        names.push(name);
    }
    names.sort_unstable();
    Ok(sequence(names.into_iter().map(string).collect()))
}

fn getenv(name: &str) -> Result<V, IoFault> {
    if name.is_empty() || name.contains(['=', '\0']) {
        return Err(IoFault::details(
            IoKind::Read,
            name,
            format!("invalid environment variable name {name:?}"),
        ));
    }
    match std::env::var(name) {
        Ok(value) => Ok(optional(Some(string(value)))),
        Err(std::env::VarError::NotPresent) => Ok(optional(None)),
        Err(std::env::VarError::NotUnicode(_)) => Err(IoFault::encoding(name)),
    }
}

fn text_case(s: &str, upper: bool, span: Span) -> R<V> {
    if s.len() > TEXT_LIMIT {
        return Err(error(span, "case conversion input exceeds the text limit"));
    }
    let value = if upper {
        s.to_uppercase()
    } else {
        s.to_lowercase()
    };
    if value.len() > TEXT_LIMIT {
        return Err(error(span, "case conversion output exceeds the text limit"));
    }
    Ok(string(value))
}

fn text_split(separator: &str, input: &str, span: Span) -> R<V> {
    if input.len() > TEXT_LIMIT {
        return Err(error(span, "split input exceeds the text limit"));
    }
    let mut parts = Vec::new();
    for item in input.split(separator) {
        if parts.len() >= COLLECTION_LIMIT {
            return Err(error(span, "split result exceeds the collection limit"));
        }
        parts.push(string(item));
    }
    Ok(sequence(parts))
}

fn text_join(separator: &str, input: &V, span: Span) -> R<V> {
    let seq = as_seq(input).map_err(|e| invariant(span, e))?;
    let mut remaining = TEXT_LIMIT
        .checked_sub(separator.len().saturating_mul(seq.len().saturating_sub(1)))
        .ok_or_else(|| error(span, "join result exceeds the text limit"))?;
    let mut parts = Vec::with_capacity(seq.len());
    for value in seq {
        let V::String(part) = value else {
            return Err(invariant(span, "str.join requires a sequence of strings"));
        };
        remaining = remaining
            .checked_sub(part.len())
            .ok_or_else(|| error(span, "join result exceeds the text limit"))?;
        parts.push(part.as_ref());
    }
    Ok(string(parts.join(separator)))
}

fn typed_failure(
    program: &HirProgram,
    owner: &str,
    tag: &str,
    payload: &str,
    detail: impl Into<String>,
    span: Span,
) -> R<V> {
    Ok(operations041::failure(
        operations041::variant(program, owner, tag, vec![string(payload)], span)?,
        span,
        Some(detail.into()),
    ))
}

fn regex_find(pattern: &str, input: &str, program: &HirProgram, span: Span) -> R<V> {
    let bad_pattern = |detail| typed_failure(program, "rxerr", "badpattern", pattern, detail, span);
    if pattern.len() > REGEX_PATTERN_LIMIT {
        return bad_pattern("regular expression exceeds the pattern byte limit".into());
    }
    if input.len() > TEXT_LIMIT {
        return Err(error(
            span,
            "regular expression input exceeds the text limit",
        ));
    }
    let regex = match RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .nest_limit(128)
        .build()
    {
        Ok(re) => re,
        Err(failure) => return bad_pattern(failure.to_string()),
    };
    Ok(succeeded(optional(
        regex.find(input).map(|m| string(m.as_str())),
    )))
}

// Civil date ↔ Unix-day transformations. Use Euclidean division so the
// proleptic calendar remains symmetric for dates before the epoch.
fn days_from_date(year: i64, month: i64, day: i64) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let march_month = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * march_month + 2) / 5 + day - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

fn date_from_days(days: i64) -> (i64, i64, i64) {
    let absolute = days + 719_468;
    let era = absolute.div_euclid(146_097);
    let doe = absolute.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let march_month = (5 * doy + 2) / 153;
    let day = doy - (153 * march_month + 2) / 5 + 1;
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    };
    (y + i64::from(month <= 2), month, day)
}

fn iso_days(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digits = |xs: &[u8]| {
        xs.iter().try_fold(0i64, |n, &b| {
            b.is_ascii_digit().then(|| n * 10 + i64::from(b - b'0'))
        })
    };
    let (y, m, d) = (digits(&b[..4])?, digits(&b[5..7])?, digits(&b[8..])?);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_len = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 28 + i64::from(leap),
        _ => return None,
    };
    (y > 0 && 1 <= d && d <= month_len).then(|| days_from_date(y, m, d))
}

fn iso_date(days: i64) -> Option<String> {
    if !(days_from_date(1, 1, 1)..=days_from_date(9999, 12, 31)).contains(&days) {
        return None;
    }
    let (y, m, d) = date_from_days(days);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

fn day_index(value: &V) -> Option<i64> {
    if let V::Exact(n) = value {
        i64::try_from(n.integral().ok()?).ok()
    } else {
        wide05::integral(value).ok()?.to_i64()
    }
}

pub(super) fn apply_core(op: CoreOp, args: &[V], span: Span, program: &HirProgram) -> Option<R<V>> {
    use CoreOp::*;
    Some(match (op, args) {
        (ReadLine | SysArgs | SysPid, []) | (FsList | FsMkdir | SysEnv, [V::String(_)]) => {
            Ok(V::Action(Rc::new(Action::Library(op, args.to_vec()))))
        }
        (FsBase, [V::String(path)]) => Ok(optional(
            Path::new(path.as_ref())
                .file_name()
                .and_then(|name| name.to_str())
                .map(string),
        )),
        (FsJoin, [V::String(path), V::String(part)]) => {
            let path = Path::new(path.as_ref()).join(part.as_ref());
            path.to_str()
                .map(string)
                .ok_or_else(|| invariant(span, "joined UTF-8 paths produced a non-UTF-8 path"))
        }
        (StrUpper, [V::String(s)]) => text_case(s, true, span),
        (StrLower, [V::String(s)]) => text_case(s, false, span),
        (StrTrim, [V::String(s)]) => Ok(string(s.trim())),
        (StrSplit, [V::String(delimiter), V::String(s)]) => text_split(delimiter, s, span),
        (StrJoin, [V::String(delimiter), seq]) => text_join(delimiter, seq, span),
        (RxFind, [V::String(pattern), V::String(input)]) => {
            regex_find(pattern, input, program, span)
        }
        (DtDays, [V::String(iso)]) => match iso_days(iso) {
            Some(days) => Ok(succeeded(exact_i128(days.into(), Domain::Integer))),
            None => typed_failure(
                program,
                "dterr",
                "baddate",
                iso,
                "invalid Gregorian date (expected YYYY-MM-DD, year 0001–9999)",
                span,
            ),
        },
        (DtDate, [value]) => match day_index(value).and_then(iso_date) {
            Some(iso) => Ok(succeeded(string(iso))),
            None => typed_failure(
                program,
                "dterr",
                "baddate",
                &repr(value),
                "day is outside the supported Gregorian date range (years 0001–9999)",
                span,
            ),
        },
        (
            ReadLine | FsList | FsMkdir | SysArgs | SysPid | SysEnv | FsBase | FsJoin | StrUpper
            | StrLower | StrTrim | StrSplit | StrJoin | RxFind | DtDays | DtDate,
            _,
        ) => Err(invariant(span, "invalid standard-library operands")),
        _ => return None,
    })
}

pub(super) fn apply_action(action: &Action, eval: &mut Eval<'_>, span: Span) -> Option<R<V>> {
    let Action::Library(op, args) = action else {
        return None;
    };
    use CoreOp::*;
    Some(match (*op, args.as_slice()) {
        (ReadLine, []) => operations041::on_io(eval.p, read_line(eval.stdin), span),
        (FsList, [V::String(path)]) => operations041::on_io(eval.p, directory(path), span),
        (FsMkdir, [V::String(path)]) => operations041::on_io(
            eval.p,
            std::fs::create_dir(path.as_ref())
                .map(|()| V::Unit)
                .map_err(|e| IoFault::system(path, e, true)),
            span,
        ),
        (SysArgs, []) => Ok(sequence(eval.args.iter().map(string).collect())),
        (SysPid, []) => Ok(exact_i128(std::process::id().into(), Domain::Natural)),
        (SysEnv, [V::String(name)]) => operations041::on_io(eval.p, getenv(name), span),
        _ => Err(invariant(
            span,
            "invalid suspended standard-library operation",
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn line_eof_crlf_and_encoding_have_distinct_outcomes() {
        let mut input = Cursor::new(&b"one\r\n\n\xc3\xa9\nlast\r"[..]);
        for expected in ["one", "", "é", "last\r"] {
            let V::Optional(Some(value)) = read_line(&mut input).unwrap() else {
                panic!("expected a line");
            };
            assert!(matches!(&*value, V::String(s) if s.as_ref() == expected));
        }
        assert!(matches!(read_line(&mut input), Ok(V::Optional(None))));
        let err = read_line(&mut Cursor::new([0xff, b'\n']))
            .err()
            .expect("expected invalid UTF-8");
        assert_eq!(err.kind, IoKind::Encoding);
        assert_eq!(err.payload, "stdin");
    }

    #[test]
    fn line_resource_limit_is_byte_based_and_bounded() {
        let mut data = vec![b'x'; LINE_LIMIT];
        data.extend_from_slice(b"\r\n");
        let mut input = Cursor::new(&data);
        assert!(matches!(read_line(&mut input), Ok(V::Optional(Some(_)))));
        data.resize(LINE_LIMIT + 40, b'x');
        data[LINE_LIMIT..].fill(b'x');
        let mut input = Cursor::new(&data);
        assert_eq!(
            read_line(&mut input)
                .err()
                .expect("expected line limit")
                .kind,
            IoKind::Read
        );
        assert_eq!(input.position() as usize, LINE_LIMIT + 2);
    }

    #[test]
    fn dates_strictly_round_trip_boundaries_and_leap_days() {
        for (date, days) in [
            ("0001-01-01", -719_162),
            ("1600-02-29", -135_081),
            ("1900-03-01", -25_508),
            ("1969-12-31", -1),
            ("1970-01-01", 0),
            ("2000-02-29", 11_016),
            ("9999-12-31", 2_932_896),
        ] {
            assert_eq!(iso_days(date), Some(days), "{date}");
            assert_eq!(iso_date(days).as_deref(), Some(date));
        }
        for malformed in [
            "0000-01-01",
            "2026-2-01",
            "1900-02-29",
            "2026-12-32",
            "2026-13-01",
            "é-é",
        ] {
            assert!(iso_days(malformed).is_none(), "{malformed}");
        }
        for out_of_range in [-719_163, 2_932_897, i64::MIN, i64::MAX] {
            assert!(iso_date(out_of_range).is_none());
        }
    }
}
