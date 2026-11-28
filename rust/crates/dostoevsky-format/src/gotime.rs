//! Times as Go's `time.Time` reads them from JSON (`Time.UnmarshalJSON`).
//!
//! Go first tries a strict RFC 3339 reader and, when that refuses, falls back to its general layout parser with the
//! RFC 3339 layout; the extra checks written after the fallback are switched off in the Go source. What is accepted
//! is therefore what the general parser accepts, which is more than RFC 3339: an hour of one digit, a comma before
//! the fraction of a second, a zone offset of up to 24 hours and 60 minutes. The legacy journal's validity depends on
//! this, so the same inputs are accepted here, and the tests check that against the Go function.

/// An instant: seconds since the Unix epoch and the nanoseconds within the second. Years 0 to 9999 fit; most of
/// that range is outside what an i64 of nanoseconds can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant {
    pub seconds: i64,
    pub nanoseconds: u32,
}

fn digit(s: &[u8], i: usize) -> Option<i64> {
    s.get(i).filter(|c| c.is_ascii_digit()).map(|c| (c - b'0') as i64)
}

/// Exactly two digits. Go's `getnum(s, true)`.
fn two(s: &[u8]) -> Option<(i64, &[u8])> {
    Some((digit(s, 0)? * 10 + digit(s, 1)?, &s[2..]))
}

fn expect(s: &[u8], c: u8) -> Option<&[u8]> {
    (s.first() == Some(&c)).then(|| &s[1..])
}

fn days_in(month: i64, year: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to the given date of the proleptic Gregorian calendar.
fn days_from_epoch(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y.rem_euclid(400);
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The text between the quotes of a JSON time, as `Time.UnmarshalJSON` reads it. `None` where Go reports an error.
pub fn parse_rfc3339(text: &[u8]) -> Option<Instant> {
    let mut s = text;
    // The year is exactly four digits.
    let year = digit(s, 0)? * 1000 + digit(s, 1)? * 100 + digit(s, 2)? * 10 + digit(s, 3)?;
    s = expect(&s[4..], b'-')?;
    let (month, rest) = two(s)?;
    if !(1..=12).contains(&month) {
        return None;
    }
    s = expect(rest, b'-')?;
    let (day, rest) = two(s)?;
    s = expect(rest, b'T')?;
    // The hour may have one digit: the general parser reads "15" as one or two.
    let mut hour = digit(s, 0)?;
    s = &s[1..];
    if let Some(d) = digit(s, 0) {
        hour = hour * 10 + d;
        s = &s[1..];
    }
    if hour >= 24 {
        return None;
    }
    s = expect(s, b':')?;
    let (minute, rest) = two(s)?;
    if minute >= 60 {
        return None;
    }
    s = expect(rest, b':')?;
    let (second, rest) = two(s)?;
    if second >= 60 {
        return None;
    }
    s = rest;
    // A fraction of a second after a period or a comma: any number of digits, the first nine kept.
    let mut nanoseconds = 0u32;
    if matches!(s.first(), Some(b'.' | b',')) && digit(s, 1).is_some() {
        let mut n = 1;
        let mut scale = 100_000_000u32;
        while let Some(d) = digit(s, n) {
            nanoseconds += d as u32 * scale;
            scale /= 10;
            n += 1;
        }
        s = &s[n..];
    }
    let offset = if s.first() == Some(&b'Z') {
        s = &s[1..];
        0
    } else {
        if s.len() < 6 || s[3] != b':' {
            return None;
        }
        let (hours, _) = two(&s[1..3])?;
        let (minutes, _) = two(&s[4..6])?;
        // Go tests with "greater than": an offset of 24 hours or of 60 minutes is accepted.
        if hours > 24 || minutes > 60 {
            return None;
        }
        let seconds = (hours * 60 + minutes) * 60;
        let signed = match s[0] {
            b'+' => seconds,
            b'-' => -seconds,
            _ => return None,
        };
        s = &s[6..];
        signed
    };
    if !s.is_empty() || day < 1 || day > days_in(month, year) {
        return None;
    }
    let seconds = days_from_epoch(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    Some(Instant { seconds, nanoseconds })
}
