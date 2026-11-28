//! Binary frame, format version 3. Mirrors `frame3.go`.
//!
//! The segmented store uses these frames for journals and checkpoints.
//!
//! ```text
//! 0   4  length of the body (u32)
//! 4   1  type
//! 5   1  flags (reserved, must be zero)
//! 6   4  header check: CRC-32C of bytes 0 to 5
//! 10  4  body check: CRC-32C of the body, seeded with the header check
//! 14  .. body
//! ```
//!
//! The header check lets a reader trust the length before it has the body. That matters at the end of a file: "the
//! data ends inside this frame" is a torn append, safe to truncate, only if the length that says so is itself
//! verified; a damaged length field must never look like a torn tail, or truncating it would destroy acknowledged
//! data. So:
//!
//! - fewer than 14 bytes left: [`Frame3Error::Short`] (a torn header; at most a partial header is lost);
//! - a complete header whose check fails: [`Frame3Error::Header`] (corruption, never a torn tail);
//! - a verified header declaring more body than the data holds: [`Frame3Error::Short`] (a torn append);
//! - a complete frame whose body check fails: [`Frame3Error::Checksum`] (corruption).
//!
//! The body check is seeded with the header check, so a body cannot be moved under another valid header. All
//! integers are little-endian. A body is at most [`FRAME3_MAX`] bytes; the writer refuses to produce more and the
//! reader refuses a declared length above it.
//!
//! Every `append_` function leaves `dst` as it was when it returns an error.

use crate::crc32c;
use crate::gotime::Instant;
use std::fmt;

pub const FRAME3_HEADER: usize = 14;
/// The same limit as the JSON frame of the legacy journal (`maxFrame` in `journal.go`): 4 MiB of body.
pub const FRAME3_MAX: usize = 4 << 20;

pub const F3_RECORD: u8 = 1;
pub const F3_COMMIT: u8 = 2;
pub const F3_TOPIC_DEF: u8 = 4;
pub const F3_FENCE: u8 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame3Error {
    /// Go's `errFrame3Short`.
    Short,
    /// Go's `errFrame3Header`.
    Header,
    /// Go's `errFrame3Checksum`.
    Checksum,
    /// Go's `errFrame3Invalid`, with what follows it in the Go message.
    Invalid(String),
    /// Go's `ErrLimit`: the writer was asked for more than the format holds.
    Limit,
}

impl fmt::Display for Frame3Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Frame3Error::Short => f.write_str("frame3: truncated"),
            Frame3Error::Header => f.write_str("frame3: header check failed"),
            Frame3Error::Checksum => f.write_str("frame3: checksum mismatch"),
            Frame3Error::Invalid(what) => write!(f, "frame3: invalid frame: {what}"),
            Frame3Error::Limit => f.write_str("size limit exceeded"),
        }
    }
}

impl std::error::Error for Frame3Error {}

fn invalid<T>(what: String) -> Result<T, Frame3Error> {
    Err(Frame3Error::Invalid(what))
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("four bytes"))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("eight bytes"))
}

/// Appends a frame header for a body of `body` bytes, with both checks still zero.
fn open(dst: &mut Vec<u8>, typ: u8, body: usize) -> usize {
    let start = dst.len();
    dst.extend_from_slice(&(body as u32).to_le_bytes());
    dst.extend_from_slice(&[typ, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    start
}

/// Appends one frame holding `body` to `dst`.
pub fn append_frame3(dst: &mut Vec<u8>, typ: u8, body: &[u8]) -> Result<(), Frame3Error> {
    if body.len() > FRAME3_MAX {
        return Err(Frame3Error::Limit);
    }
    let start = open(dst, typ, body.len());
    dst.extend_from_slice(body);
    seal(&mut dst[start..]);
    Ok(())
}

/// Fills in the two checks of the frame that starts at `dst[start..]`, and reports whether there was a frame's
/// header there to fill in. With fewer than [`FRAME3_HEADER`] bytes from `start`, or a `start` beyond the end, it
/// changes nothing and returns false. Go's `sealFrame3` is unexported and panics on such a slice; this function is
/// public, so it checks.
#[must_use = "false means nothing was sealed"]
pub fn seal_frame3(dst: &mut [u8], start: usize) -> bool {
    match dst.get_mut(start..) {
        Some(f) if f.len() >= FRAME3_HEADER => {
            seal(f);
            true
        }
        _ => false,
    }
}

/// The two checks of the frame `f`, which holds at least its header.
fn seal(f: &mut [u8]) {
    let hcrc = crc32c::checksum(&f[..6]);
    f[6..10].copy_from_slice(&hcrc.to_le_bytes());
    let bcrc = crc32c::update(hcrc, &f[FRAME3_HEADER..]);
    f[10..14].copy_from_slice(&bcrc.to_le_bytes());
}

/// Checks the frame at the start of `b` and returns its type, its body (a view into `b`) and the frame's total
/// size. It never reads past `b` and never allocates on success.
pub fn read_frame3(b: &[u8]) -> Result<(u8, &[u8], usize), Frame3Error> {
    if b.len() < FRAME3_HEADER {
        return Err(Frame3Error::Short);
    }
    let hcrc = crc32c::checksum(&b[..6]);
    if hcrc != u32_at(b, 6) {
        return Err(Frame3Error::Header);
    }
    let n = u32_at(b, 0) as usize;
    if n > FRAME3_MAX {
        return invalid(format!("length {n}"));
    }
    if b[5] != 0 {
        return invalid(format!("flags {:#x}", b[5]));
    }
    let size = FRAME3_HEADER + n;
    if b.len() < size {
        return Err(Frame3Error::Short); // the header is verified, so the data really does end inside this frame
    }
    if crc32c::update(hcrc, &b[FRAME3_HEADER..size]) != u32_at(b, 10) {
        return Err(Frame3Error::Checksum);
    }
    Ok((b[4], &b[FRAME3_HEADER..size], size))
}

// ----- record -----

/// The time field of a record is Unix nanoseconds in an i64. This value is reserved: it stands for Go's zero
/// `time.Time`. The instants a frame can hold are therefore those from 1677-09-21 00:12:43.145224193 UTC through
/// 2262-04-11 23:47:16.854775807 UTC, inclusive.
///
/// The Go engine carries a `time.Time` and converts at the frame. This engine carries the nanoseconds themselves
/// and converts where a time arrives in another form, which is the legacy journal's text: [`time_to_nanos`] and
/// [`nanos_to_time`].
pub const ZERO_TIME: i64 = i64::MIN;

/// Go's zero `time.Time`: January 1 of year 1, 00:00:00 UTC.
pub const ZERO_INSTANT: Instant = Instant {
    seconds: -62_135_596_800,
    nanoseconds: 0,
};

/// An instant that a frame's time field cannot hold. Go's `timeToNanos` refuses it with an error that wraps nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeOutOfRange(pub Instant);

impl fmt::Display for TimeOutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "time {}.{:09} (seconds since the Unix epoch) is outside the range of a nanosecond timestamp",
            self.0.seconds, self.0.nanoseconds
        )
    }
}

impl std::error::Error for TimeOutOfRange {}

/// Converts an instant to the frame's time field. The zero time is kept as [`ZERO_TIME`]; an instant outside the
/// range a frame can hold is refused. Mirrors `timeToNanos`.
pub fn time_to_nanos(t: Instant) -> Result<i64, TimeOutOfRange> {
    if t == ZERO_INSTANT {
        return Ok(ZERO_TIME);
    }
    // In i128 nothing can overflow: the seconds of any instant times 10^9 fit with room to spare.
    let nanos = i128::from(t.seconds) * 1_000_000_000 + i128::from(t.nanoseconds);
    match i64::try_from(nanos) {
        Ok(n) if n != ZERO_TIME && t.nanoseconds < 1_000_000_000 => Ok(n),
        _ => Err(TimeOutOfRange(t)),
    }
}

/// Converts the frame's time field to an instant. Mirrors `nanosToTime`.
pub fn nanos_to_time(n: i64) -> Instant {
    if n == ZERO_TIME {
        return ZERO_INSTANT;
    }
    Instant {
        seconds: n.div_euclid(1_000_000_000),
        nanoseconds: n.rem_euclid(1_000_000_000) as u32,
    }
}

/// A Record frame's content. Key, ID and value borrow the buffer they were decoded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Record3<'a> {
    /// The partition's id; a topic-define frame maps it to a topic and number.
    pub partition: u32,
    pub offset: u64,
    /// Unix nanoseconds; [`ZERO_TIME`] stands for the zero time.
    pub time: i64,
    pub key: &'a [u8],
    pub id: &'a [u8],
    pub value: &'a [u8],
}

const RECORD3_FIXED: usize = 4 + 8 + 8 + 2 + 2;

/// Appends a Record frame. Key and ID are limited to 65,535 bytes (the engine's own limits are far lower); the
/// frame to [`FRAME3_MAX`].
pub fn append_record3(dst: &mut Vec<u8>, r: &Record3) -> Result<(), Frame3Error> {
    if r.key.len() > u16::MAX as usize || r.id.len() > u16::MAX as usize {
        return Err(Frame3Error::Limit);
    }
    let body = RECORD3_FIXED + r.key.len() + r.id.len() + r.value.len();
    if body > FRAME3_MAX {
        return Err(Frame3Error::Limit);
    }
    dst.reserve(FRAME3_HEADER + body);
    let start = open(dst, F3_RECORD, body);
    dst.extend_from_slice(&r.partition.to_le_bytes());
    dst.extend_from_slice(&r.offset.to_le_bytes());
    dst.extend_from_slice(&(r.time as u64).to_le_bytes());
    dst.extend_from_slice(&(r.key.len() as u16).to_le_bytes());
    dst.extend_from_slice(&(r.id.len() as u16).to_le_bytes());
    dst.extend_from_slice(r.key);
    dst.extend_from_slice(r.id);
    dst.extend_from_slice(r.value);
    seal(&mut dst[start..]);
    Ok(())
}

/// Decodes a Record frame's body. The value takes whatever follows the key and the ID, so a body whose declared
/// lengths exceed it is refused.
pub fn parse_record3(body: &[u8]) -> Result<Record3<'_>, Frame3Error> {
    if body.len() < RECORD3_FIXED {
        return invalid(format!("record body of {} bytes", body.len()));
    }
    let (k, id) = (u16_at(body, 20) as usize, u16_at(body, 22) as usize);
    let rest = &body[RECORD3_FIXED..];
    if k + id > rest.len() {
        return invalid(format!(
            "key {k} and id {id} exceed the {} bytes that follow",
            rest.len()
        ));
    }
    Ok(Record3 {
        partition: u32_at(body, 0),
        offset: u64_at(body, 4),
        time: u64_at(body, 12) as i64,
        key: &rest[..k],
        id: &rest[k..k + id],
        value: &rest[k + id..],
    })
}

// ----- commit -----

/// A consumer-group commit: the group's next offset for a partition, the optional offset the commit expects the
/// group to be at, and the processor state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Commit3<'a> {
    pub partition: u32,
    pub group: &'a [u8],
    pub next: u64,
    /// Go's `HasExpected` and `Expected`.
    pub expected: Option<u64>,
    pub state: &'a [u8],
}

pub fn append_commit3(dst: &mut Vec<u8>, c: &Commit3) -> Result<(), Frame3Error> {
    if c.group.len() > u16::MAX as usize {
        return Err(Frame3Error::Limit);
    }
    let body = 4 + 2 + c.group.len() + 8 + 1 + 8 + c.state.len();
    if body > FRAME3_MAX {
        return Err(Frame3Error::Limit);
    }
    let start = open(dst, F3_COMMIT, body);
    dst.extend_from_slice(&c.partition.to_le_bytes());
    dst.extend_from_slice(&(c.group.len() as u16).to_le_bytes());
    dst.extend_from_slice(c.group);
    dst.extend_from_slice(&c.next.to_le_bytes());
    dst.push(c.expected.is_some() as u8);
    dst.extend_from_slice(&c.expected.unwrap_or(0).to_le_bytes()); // fixed width: zero when absent
    dst.extend_from_slice(c.state);
    seal(&mut dst[start..]);
    Ok(())
}

pub fn parse_commit3(body: &[u8]) -> Result<Commit3<'_>, Frame3Error> {
    if body.len() < 4 + 2 {
        return invalid(format!("commit body of {} bytes", body.len()));
    }
    let g = u16_at(body, 4) as usize;
    let rest = &body[6..];
    if rest.len() < g + 8 + 1 + 8 {
        return invalid(format!("commit group of {g} bytes exceeds the body"));
    }
    let (group, rest) = rest.split_at(g);
    let expected = match rest[8] {
        0 => {
            if u64_at(rest, 9) != 0 {
                return invalid("absent expected offset is not zero".into());
            }
            None
        }
        1 => Some(u64_at(rest, 9)),
        flag => return invalid(format!("expected-offset flag {flag}")),
    };
    Ok(Commit3 {
        partition: u32_at(body, 0),
        group,
        next: u64_at(rest, 0),
        expected,
        state: &rest[17..],
    })
}

// ----- topic definition -----

/// Maps a partition id to its topic and number, once, so a record frame carries a small integer instead of a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TopicDef3<'a> {
    pub partition: u32,
    pub number: u32,
    pub topic: &'a [u8],
}

pub fn append_topic_def3(dst: &mut Vec<u8>, d: &TopicDef3) -> Result<(), Frame3Error> {
    if d.topic.len() > u16::MAX as usize {
        return Err(Frame3Error::Limit);
    }
    let start = open(dst, F3_TOPIC_DEF, 4 + 4 + d.topic.len());
    dst.extend_from_slice(&d.partition.to_le_bytes());
    dst.extend_from_slice(&d.number.to_le_bytes());
    dst.extend_from_slice(d.topic);
    seal(&mut dst[start..]);
    Ok(())
}

pub fn parse_topic_def3(body: &[u8]) -> Result<TopicDef3<'_>, Frame3Error> {
    if body.len() < 8 {
        return invalid(format!("topic definition of {} bytes", body.len()));
    }
    Ok(TopicDef3 {
        partition: u32_at(body, 0),
        number: u32_at(body, 4),
        topic: &body[8..],
    })
}

#[cfg(test)]
mod tests {
    //! The properties `frame3_test.go` checks. The Go functions' own results on several thousand inputs are checked
    //! in `tests/go_vectors.rs`.
    use super::*;

    fn sample() -> Vec<u8> {
        let mut f = Vec::new();
        let r = Record3 {
            partition: 7,
            offset: 41,
            time: 1_790_000_000_000_000_000,
            key: b"key",
            id: b"id-1",
            value: b"a value of some length",
        };
        append_record3(&mut f, &r).unwrap();
        f
    }

    #[test]
    fn every_single_bit_flip_is_detected() {
        let good = sample();
        for bit in 0..good.len() * 8 {
            let mut bad = good.clone();
            bad[bit / 8] ^= 1 << (bit % 8);
            let got = read_frame3(&bad);
            assert!(got.is_err(), "bit {bit} went unnoticed");
            // A flip in the first ten bytes damages the header, and a damaged header is never a torn tail.
            if bit < 80 {
                assert_eq!(got, Err(Frame3Error::Header), "bit {bit}");
            }
        }
    }

    #[test]
    fn truncation_at_every_length_is_a_short_frame() {
        let good = sample();
        for n in 0..good.len() {
            assert_eq!(read_frame3(&good[..n]), Err(Frame3Error::Short), "length {n}");
        }
        let mut longer = good.clone();
        longer.extend_from_slice(b"what follows");
        assert_eq!(read_frame3(&longer).map(|f| f.2), Ok(good.len()));
    }

    #[test]
    fn a_fence_is_a_frame_with_no_body() {
        let mut f = Vec::new();
        append_frame3(&mut f, F3_FENCE, b"").unwrap();
        assert_eq!(
            (f.len(), read_frame3(&f)),
            (FRAME3_HEADER, Ok((F3_FENCE, &b""[..], FRAME3_HEADER)))
        );
    }

    #[test]
    fn sealing_checks_that_a_header_is_there() {
        let mut f = Vec::new();
        append_frame3(&mut f, 9, b"body").unwrap();
        let sealed = f.clone();
        f[6..14].fill(0);
        assert!(read_frame3(&f).is_err());
        assert!(seal_frame3(&mut f, 0));
        assert_eq!(f, sealed, "sealing writes the two checks and nothing else");
        // A second frame after the first, sealed in place.
        let mut two = [sealed.clone(), sealed.clone()].concat();
        two[sealed.len() + 6..sealed.len() + 14].fill(0);
        assert!(seal_frame3(&mut two, sealed.len()));
        assert_eq!(two, [sealed.clone(), sealed.clone()].concat());
        // Too little to be a header, and a start at or beyond the end: nothing is written.
        for (length, start) in [(13, 0), (0, 0), (14, 1), (14, 14), (14, 15), (30, usize::MAX)] {
            let mut short = vec![7u8; length];
            assert!(!seal_frame3(&mut short, start), "{length} bytes from {start}");
            assert_eq!(short, vec![7u8; length]);
        }
        let mut exact = vec![0u8; FRAME3_HEADER];
        assert!(
            seal_frame3(&mut exact, 0),
            "a header alone is a frame with an empty body"
        );
        assert_eq!(
            read_frame3(&exact).map(|(_, body, size)| (body.len(), size)),
            Ok((0, FRAME3_HEADER))
        );
    }

    #[test]
    fn the_zero_time_is_the_smallest_value() {
        let mut f = Vec::new();
        append_record3(
            &mut f,
            &Record3 {
                time: ZERO_TIME,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(parse_record3(read_frame3(&f).unwrap().1).unwrap().time, i64::MIN);
    }
}
