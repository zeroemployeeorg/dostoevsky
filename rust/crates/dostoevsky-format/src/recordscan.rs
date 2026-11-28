//! Reads the four fields replay needs from a Record frame of the legacy journal without decoding the frame.
//! Mirrors `recordscan.go`.
//!
//! A Record frame is Go's `json.Marshal` of an entry holding one record, so its shape is fixed:
//!
//! ```text
//! {"Record":{"Topic":"t","Number":0,"Key":"k","Value":"<base64>","ID":"i","Offset":7,"Time":"..."}}
//! ```
//!
//! (`Value` is `null` for a nil slice.) The scanner accepts only exactly that shape. For anything else it returns
//! `None` and the caller decodes the frame as JSON, so a frame the scanner does not fully understand behaves exactly
//! as before, including its error.
//!
//! The scanner accepts a frame only if the JSON decoder would accept it, so open, repair and read agree on what is
//! damaged. The value is therefore decoded (into a buffer that is reused, and thrown away) and the time is parsed.

use crate::{base64, gotime};

/// What a scan found. The strings borrow the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scanned<'a> {
    pub topic: &'a str,
    pub number: i64,
    pub offset: i64,
    /// Empty when the record has no ID.
    pub id: &'a str,
}

#[derive(Debug, Default)]
pub struct RecordScanner {
    /// Scratch for decoding values.
    value: Vec<u8>,
}

const RECORD_PREFIX: &[u8] = b"{\"Record\":{\"Topic\":\"";

/// The bytes of a JSON string that starts after its opening quote at `b[i]`, and the index after the closing quote,
/// when the string is plain: no escape, no control byte, valid UTF-8. A plain string decodes to exactly its bytes.
fn plain_string(b: &[u8], i: usize) -> Option<(&str, usize)> {
    for (n, &c) in b.get(i..)?.iter().enumerate() {
        match c {
            b'"' => return std::str::from_utf8(&b[i..i + n]).ok().map(|s| (s, i + n + 1)),
            b'\\' | 0x00..=0x1f => return None,
            _ => {}
        }
    }
    None
}

/// A non-negative integer written the way Go's encoder writes one: digits only, no sign, no leading zero, at most
/// 18 digits (it fits an i64).
fn canonical_uint(b: &[u8], start: usize) -> Option<(i64, usize)> {
    let mut v = 0i64;
    let mut i = start;
    while let Some(c) = b.get(i).filter(|c| c.is_ascii_digit()) {
        v = v.wrapping_mul(10).wrapping_add((c - b'0') as i64);
        if i - start >= 18 {
            return None;
        }
        i += 1;
    }
    let n = i - start;
    if n == 0 || (n > 1 && b[start] == b'0') {
        return None;
    }
    Some((v, i))
}

fn expect(b: &[u8], i: usize, literal: &[u8]) -> Option<usize> {
    b.get(i..)?.starts_with(literal).then_some(i + literal.len())
}

impl RecordScanner {
    pub fn scan<'a>(&mut self, b: &'a [u8]) -> Option<Scanned<'a>> {
        let i = expect(b, 0, RECORD_PREFIX)?;
        let (topic, i) = plain_string(b, i)?;
        let i = expect(b, i, b",\"Number\":")?;
        let (number, i) = canonical_uint(b, i)?;
        let i = expect(b, i, b",\"Key\":\"")?;
        let (_, i) = plain_string(b, i)?;
        let i = expect(b, i, b",\"Value\":")?;
        let i = match expect(b, i, b"null") {
            Some(j) => j,
            None => {
                let i = expect(b, i, b"\"")?;
                let end = b[i..].iter().position(|&c| c == b'"')?;
                let value = &b[i..i + end];
                // The decoder skips line breaks, which unescaped in a JSON string are not JSON; every other byte
                // that JSON would refuse here, or that needs unescaping, is outside the alphabet and fails the
                // decode.
                if value.contains(&b'\n') || value.contains(&b'\r') || !base64::decode_into(value, &mut self.value) {
                    return None;
                }
                i + end + 1
            }
        };
        let i = expect(b, i, b",\"ID\":\"")?;
        let (id, i) = plain_string(b, i)?;
        let i = expect(b, i, b",\"Offset\":")?;
        let (offset, i) = canonical_uint(b, i)?;
        let i = expect(b, i, b",\"Time\":\"")?;
        let (time, i) = plain_string(b, i)?;
        gotime::parse_rfc3339(time.as_bytes())?;
        if expect(b, i, b"}}")? != b.len() {
            return None;
        }
        Some(Scanned {
            topic,
            number,
            offset,
            id,
        })
    }
}
