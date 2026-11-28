//! Strict Gregorian date parsing and civil-day conversion.
use super::super::{
    exact_i128, invariant, repr, succeeded, wide, CoreOp, Domain, HirProgram, Span, R, V,
};
use super::{string, typed_failure};
use num_traits::ToPrimitive;

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
        wide::integral(value).ok()?.to_i64()
    }
}

pub(super) fn apply(op: CoreOp, args: &[V], program: &HirProgram, span: Span) -> R<V> {
    use CoreOp::*;
    match (op, args) {
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
        _ => Err(invariant(span, "invalid standard-library operands")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
