//! The byte layout of the wire protocol, version 1, as specified in `docs/wire-v1.md`. Mirrors `wire/wire.go`.
//!
//! It encodes and decodes the handshake and frames for both sides and enforces the protocol's bounds. It does no
//! networking apart from reading a HELLO from a reader.
//!
//! Names, keys and message IDs are Go strings in the Go package: bytes that are checked for UTF-8 only where the
//! protocol says so. Here they are byte slices for the same reason. Decoded slices borrow the buffer they were
//! decoded from.
//!
//! Like the Go functions, a decoder returns what it decoded together with its verdict: a request that is refused
//! still carries the request ID its ERROR must answer.

use dostoevsky_format::sha256;
use std::io::Read;
use subtle::ConstantTimeEq;

pub const VERSION: u16 = 1;

pub const MAX_VALUE: usize = 1 << 20;
pub const MAX_KEY: usize = 4096;
pub const MAX_ID: usize = 256;
pub const MAX_NAME: usize = 96;
pub const MAX_TOKEN: usize = 4096;
pub const MIN_TOKEN: usize = 16;
/// Milliseconds.
pub const MAX_WAIT: u16 = 30000;
pub const MAX_BODY: usize = MAX_VALUE + (8 << 10);

pub const HEADER_SIZE: usize = 10;
pub const WELCOME_SIZE: usize = 17;

/// The most a server reads before authentication.
pub const MAX_HELLO: usize = 10 + MAX_TOKEN;

const MAGIC: &[u8; 4] = b"DSTV";

/// A frame type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Type(pub u8);

impl Type {
    pub const PUBLISH: Type = Type(0x01);
    pub const READ: Type = Type(0x02);
    pub const POSITION: Type = Type(0x03);
    pub const COMMIT: Type = Type(0x04);
    pub const PING: Type = Type(0x05);

    const REPLY_BIT: u8 = 0x80;

    pub const PUBLISH_OK: Type = Type(0x01 | Type::REPLY_BIT);
    pub const READ_OK: Type = Type(0x02 | Type::REPLY_BIT);
    pub const POSITION_OK: Type = Type(0x03 | Type::REPLY_BIT);
    pub const COMMIT_OK: Type = Type(0x04 | Type::REPLY_BIT);
    pub const PING_OK: Type = Type(0x05 | Type::REPLY_BIT);

    pub const ERROR: Type = Type(0xFF);
}

/// An error code carried by ERROR frames and WELCOME. Zero is success (WELCOME only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Code(pub u8);

impl Code {
    pub const OK: Code = Code(0);
    pub const INVALID: Code = Code(1);
    pub const UNAUTHORIZED: Code = Code(2);
    pub const CONFLICT: Code = Code(3);
    pub const WRONG_OWNER: Code = Code(4);
    pub const EXPIRED: Code = Code(5);
    pub const LIMIT: Code = Code(6);
    pub const CLOSED: Code = Code(7);
    pub const TIMEOUT: Code = Code(8);
    pub const CANCELED: Code = Code(9);
    pub const STORAGE_FAILURE: Code = Code(10);
    pub const PROTOCOL: Code = Code(11);
    pub const UNSUPPORTED_VERSION: Code = Code(12);

    /// Whether this is an error code of version 1.
    fn known(self) -> bool {
        self.0 >= Code::INVALID.0 && self.0 <= Code::UNSUPPORTED_VERSION.0
    }
}

const CODE_NAMES: [&str; 13] = [
    "ok",
    "invalid",
    "unauthorized",
    "conflict",
    "wrong_owner",
    "expired",
    "limit",
    "closed",
    "timeout",
    "canceled",
    "storage_failure",
    "protocol",
    "unsupported_version",
];

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match CODE_NAMES.get(self.0 as usize) {
            Some(name) => f.write_str(name),
            None => write!(f, "code_{}", self.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A handshake or frame that does not parse. The connection cannot continue. Go's `ErrProtocol`.
    Protocol,
    /// A frame announced longer than [`MAX_BODY`], known from its header. The connection cannot continue. Go's
    /// `ErrTooLong`.
    TooLong,
    /// A well-formed request that breaks a value rule. The connection continues. Go's `*RequestError`.
    Request(Code),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Protocol => f.write_str("wire: protocol error"),
            Error::TooLong => f.write_str("wire: frame longer than the maximum"),
            Error::Request(code) => write!(f, "wire: request {code}"),
        }
    }
}

impl std::error::Error for Error {}

fn refuse(c: Code) -> Result<(), Error> {
    Err(Error::Request(c))
}

/// The fixed part of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Header {
    pub length: u32,
    pub typ: Type,
    pub flags: u8,
    pub request_id: u32,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Reads the first [`HEADER_SIZE`] bytes of `b` and checks what can be checked before the body is read or
/// allocated. The header is returned with the verdict, as in Go.
pub fn parse_header(b: &[u8]) -> (Header, Result<(), Error>) {
    if b.len() < HEADER_SIZE {
        return (Header::default(), Err(Error::Protocol));
    }
    let h = Header {
        length: u32_at(b, 0),
        typ: Type(b[4]),
        flags: b[5],
        request_id: u32_at(b, 6),
    };
    if h.length as usize > MAX_BODY {
        return (h, Err(Error::TooLong));
    }
    if h.flags != 0 {
        return (h, Err(Error::Protocol));
    }
    (h, Ok(()))
}

/// A frame from the client. Which fields mean something depends on the type: `value` is the published value or the
/// committed state, `offset` is the offset read or the expected cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Request<'a> {
    pub typ: Type,
    pub request_id: u32,
    pub topic: &'a [u8],
    pub partition: u16,
    pub offset: u64,
    pub wait_ms: u16,
    pub group: &'a [u8],
    pub key: &'a [u8],
    pub message_id: &'a [u8],
    pub value: &'a [u8],
}

/// A frame from the server. `offset` is the published or read offset, or the next position. `value` is the record
/// value or the group state. `found`, `time`, `key` and `message_id` belong to READ_OK; `code` to ERROR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reply<'a> {
    pub typ: Type,
    pub request_id: u32,
    pub partition: u16,
    pub offset: u64,
    pub found: bool,
    /// Nanoseconds since the Unix epoch.
    pub time: i64,
    pub key: &'a [u8],
    pub message_id: &'a [u8],
    pub value: &'a [u8],
    pub code: Code,
}

/// Reads fields in order; any overrun is remembered and reported once.
struct Body<'a> {
    b: &'a [u8],
    bad: bool,
}

impl<'a> Body<'a> {
    /// The next `n` bytes. After an overrun every later read is empty, as in Go.
    fn take(&mut self, n: usize) -> &'a [u8] {
        if self.bad || n > self.b.len() {
            self.bad = true;
            return &[];
        }
        let (out, rest) = self.b.split_at(n);
        self.b = rest;
        out
    }

    fn u8(&mut self) -> u8 {
        self.take(1).first().copied().unwrap_or(0)
    }

    fn u16(&mut self) -> u16 {
        match self.take(2) {
            [a, b] => u16::from_le_bytes([*a, *b]),
            _ => 0,
        }
    }

    fn u64(&mut self) -> u64 {
        self.take(8).try_into().map_or(0, u64::from_le_bytes)
    }

    fn str8(&mut self) -> &'a [u8] {
        let n = self.u8() as usize;
        self.take(n)
    }

    fn str16(&mut self) -> &'a [u8] {
        let n = self.u16() as usize;
        self.take(n)
    }

    fn rest(&mut self) -> &'a [u8] {
        self.take(self.b.len())
    }

    /// True when every field was there and nothing is left over.
    fn done(&self) -> bool {
        !self.bad && self.b.is_empty()
    }
}

fn valid_name(s: &[u8]) -> bool {
    if s.is_empty() || s.len() > MAX_NAME {
        return false;
    }
    s.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
}

fn valid_utf8(s: &[u8]) -> bool {
    std::str::from_utf8(s).is_ok()
}

impl Request<'_> {
    /// Applies the value rules to a structurally sound request.
    fn check(&self) -> Result<(), Error> {
        if self.typ == Type::PING {
            return Ok(());
        }
        if !valid_name(self.topic) {
            return refuse(Code::INVALID);
        }
        match self.typ {
            Type::PUBLISH => {
                if self.key.len() > MAX_KEY || self.message_id.len() > MAX_ID || self.value.len() > MAX_VALUE {
                    return refuse(Code::LIMIT);
                }
                if !valid_utf8(self.key) || !valid_utf8(self.message_id) {
                    return refuse(Code::INVALID);
                }
            }
            Type::READ => {
                if self.offset >= 1 << 63 || self.wait_ms > MAX_WAIT {
                    return refuse(Code::INVALID);
                }
            }
            Type::POSITION => {
                if !valid_name(self.group) {
                    return refuse(Code::INVALID);
                }
            }
            Type::COMMIT => {
                if !valid_name(self.group) || self.offset >= 1 << 63 {
                    return refuse(Code::INVALID);
                }
                if self.offset == (1 << 63) - 1 || self.value.len() > MAX_VALUE {
                    return refuse(Code::LIMIT);
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Decodes the body that follows `h`. [`Error::Protocol`] ends the connection; an [`Error::Request`] is answered
/// with an ERROR for the returned request's ID.
pub fn decode_request(h: Header, b: &[u8]) -> (Request<'_>, Result<(), Error>) {
    let mut q = Request {
        typ: h.typ,
        request_id: h.request_id,
        ..Default::default()
    };
    if h.request_id == 0 || h.flags != 0 || b.len() > MAX_BODY || b.len() as u64 != h.length as u64 {
        return (q, Err(Error::Protocol));
    }
    let mut r = Body { b, bad: false };
    match h.typ {
        Type::PUBLISH => {
            q.topic = r.str8();
            q.key = r.str16();
            q.message_id = r.str16();
            q.value = r.rest();
        }
        Type::READ => {
            q.topic = r.str8();
            q.partition = r.u16();
            q.offset = r.u64();
            q.wait_ms = r.u16();
        }
        Type::POSITION => {
            q.topic = r.str8();
            q.partition = r.u16();
            q.group = r.str8();
        }
        Type::COMMIT => {
            q.topic = r.str8();
            q.partition = r.u16();
            q.group = r.str8();
            q.offset = r.u64();
            q.value = r.rest();
        }
        Type::PING => {}
        _ => return (q, Err(Error::Protocol)),
    }
    if !r.done() {
        return (q, Err(Error::Protocol));
    }
    let verdict = q.check();
    (q, verdict)
}

fn append_str8(b: &mut Vec<u8>, s: &[u8]) {
    b.push(s.len() as u8);
    b.extend_from_slice(s);
}

fn append_str16(b: &mut Vec<u8>, s: &[u8]) {
    b.extend_from_slice(&(s.len() as u16).to_le_bytes());
    b.extend_from_slice(s);
}

/// Appends a header with a zero length; [`end`] fills the length in.
fn begin(dst: &mut Vec<u8>, t: Type, id: u32) {
    dst.extend_from_slice(&[0, 0, 0, 0, t.0, 0]);
    dst.extend_from_slice(&id.to_le_bytes());
}

fn end(dst: &mut [u8], start: usize) {
    let length = (dst.len() - start - HEADER_SIZE) as u32;
    dst[start..start + 4].copy_from_slice(&length.to_le_bytes());
}

/// Appends `q` as a frame, refusing with an [`Error::Request`] what the rules forbid. On an error `dst` is as it
/// was.
pub fn append_request(dst: &mut Vec<u8>, q: &Request) -> Result<(), Error> {
    if q.request_id == 0 || q.typ.0 < Type::PUBLISH.0 || q.typ.0 > Type::PING.0 {
        return Err(Error::Protocol);
    }
    q.check()?;
    let start = dst.len();
    begin(dst, q.typ, q.request_id);
    if q.typ != Type::PING {
        append_str8(dst, q.topic);
    }
    match q.typ {
        Type::PUBLISH => {
            append_str16(dst, q.key);
            append_str16(dst, q.message_id);
            dst.extend_from_slice(q.value);
        }
        Type::READ => {
            dst.extend_from_slice(&q.partition.to_le_bytes());
            dst.extend_from_slice(&q.offset.to_le_bytes());
            dst.extend_from_slice(&q.wait_ms.to_le_bytes());
        }
        Type::POSITION => {
            dst.extend_from_slice(&q.partition.to_le_bytes());
            append_str8(dst, q.group);
        }
        Type::COMMIT => {
            dst.extend_from_slice(&q.partition.to_le_bytes());
            append_str8(dst, q.group);
            dst.extend_from_slice(&q.offset.to_le_bytes());
            dst.extend_from_slice(q.value);
        }
        _ => {}
    }
    end(dst, start);
    Ok(())
}

/// Decodes the body that follows `h`. Whether the reply answers an outstanding request of the matching type is the
/// client's to check. Any error ends the connection.
pub fn decode_reply(h: Header, b: &[u8]) -> (Reply<'_>, Result<(), Error>) {
    let mut p = Reply {
        typ: h.typ,
        request_id: h.request_id,
        ..Default::default()
    };
    if h.flags != 0
        || b.len() > MAX_BODY
        || b.len() as u64 != h.length as u64
        || (h.request_id == 0 && h.typ != Type::ERROR)
    {
        return (p, Err(Error::Protocol));
    }
    let mut r = Body { b, bad: false };
    match h.typ {
        Type::PUBLISH_OK => {
            p.partition = r.u16();
            p.offset = r.u64();
        }
        Type::READ_OK => {
            let found = r.u8();
            if found > 1 {
                return (p, Err(Error::Protocol));
            }
            p.found = found == 1;
            if p.found {
                p.partition = r.u16();
                p.offset = r.u64();
                p.time = r.u64() as i64;
                p.key = r.str16();
                p.message_id = r.str16();
                p.value = r.rest();
                if p.key.len() > MAX_KEY
                    || p.message_id.len() > MAX_ID
                    || p.value.len() > MAX_VALUE
                    || !valid_utf8(p.key)
                    || !valid_utf8(p.message_id)
                {
                    return (p, Err(Error::Protocol));
                }
            }
        }
        Type::POSITION_OK => {
            p.offset = r.u64();
            p.value = r.rest();
            if p.value.len() > MAX_VALUE {
                return (p, Err(Error::Protocol));
            }
        }
        Type::COMMIT_OK | Type::PING_OK => {}
        Type::ERROR => {
            p.code = Code(r.u8());
            if !p.code.known() {
                return (p, Err(Error::Protocol));
            }
        }
        _ => return (p, Err(Error::Protocol)),
    }
    if !r.done() || p.offset >= 1 << 63 {
        return (p, Err(Error::Protocol));
    }
    (p, Ok(()))
}

/// Appends `p` as a frame. It refuses what [`decode_reply`] would reject. On an error `dst` is as it was.
pub fn append_reply(dst: &mut Vec<u8>, p: &Reply) -> Result<(), Error> {
    if (p.request_id == 0 && p.typ != Type::ERROR) || p.offset >= 1 << 63 {
        return Err(Error::Protocol);
    }
    if p.key.len() > MAX_KEY
        || p.message_id.len() > MAX_ID
        || p.value.len() > MAX_VALUE
        || !valid_utf8(p.key)
        || !valid_utf8(p.message_id)
    {
        return Err(Error::Protocol);
    }
    let start = dst.len();
    begin(dst, p.typ, p.request_id);
    match p.typ {
        Type::PUBLISH_OK => {
            dst.extend_from_slice(&p.partition.to_le_bytes());
            dst.extend_from_slice(&p.offset.to_le_bytes());
        }
        Type::READ_OK => {
            if !p.found {
                dst.push(0);
            } else {
                dst.push(1);
                dst.extend_from_slice(&p.partition.to_le_bytes());
                dst.extend_from_slice(&p.offset.to_le_bytes());
                dst.extend_from_slice(&(p.time as u64).to_le_bytes());
                append_str16(dst, p.key);
                append_str16(dst, p.message_id);
                dst.extend_from_slice(p.value);
            }
        }
        Type::POSITION_OK => {
            dst.extend_from_slice(&p.offset.to_le_bytes());
            dst.extend_from_slice(p.value);
        }
        Type::COMMIT_OK | Type::PING_OK => {}
        Type::ERROR if p.code.known() => dst.push(p.code.0),
        _ => {
            dst.truncate(start);
            return Err(Error::Protocol);
        }
    }
    end(dst, start);
    Ok(())
}

/// The client's first message.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hello {
    pub max_version: u16,
    pub token: Vec<u8>,
}

/// Appends a HELLO.
pub fn append_hello(dst: &mut Vec<u8>, h: &Hello) -> Result<(), Error> {
    if h.token.len() > MAX_TOKEN {
        return Err(Error::Protocol);
    }
    dst.extend_from_slice(MAGIC);
    dst.extend_from_slice(&h.max_version.to_le_bytes());
    dst.extend_from_slice(&0u16.to_le_bytes());
    dst.extend_from_slice(&(h.token.len() as u16).to_le_bytes());
    dst.extend_from_slice(&h.token);
    Ok(())
}

/// Why a HELLO could not be read.
#[derive(Debug)]
pub enum HelloError {
    /// Close without a reply.
    Protocol,
    /// The input ended before the first byte of a part of the HELLO. Go's `io.EOF`.
    Eof,
    /// The input ended inside a part of the HELLO. Go's `io.ErrUnexpectedEOF`.
    UnexpectedEof,
    Io(std::io::Error),
}

/// Go's `io.ReadFull`.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> Result<(), HelloError> {
    let mut done = 0;
    while done < buf.len() {
        match r.read(&mut buf[done..]) {
            Ok(0) => {
                return Err(if done == 0 {
                    HelloError::Eof
                } else {
                    HelloError::UnexpectedEof
                })
            }
            Ok(n) => done += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(HelloError::Io(e)),
        }
    }
    Ok(())
}

/// Reads one HELLO and never more than [`MAX_HELLO`] bytes. [`HelloError::Protocol`] means: close without a reply.
/// The caller sets the authentication deadline.
pub fn read_hello(r: &mut impl Read) -> Result<Hello, HelloError> {
    let mut fixed = [0u8; 10];
    read_full(r, &mut fixed)?;
    let n = u16_at(&fixed, 8) as usize;
    if &fixed[..4] != MAGIC || u16_at(&fixed, 6) != 0 || n > MAX_TOKEN {
        return Err(HelloError::Protocol);
    }
    let mut h = Hello {
        max_version: u16_at(&fixed, 4),
        token: vec![0; n],
    };
    read_full(r, &mut h.token)?;
    Ok(h)
}

/// Whether two digests are equal, in time that does not depend on where they differ: the comparison of the `subtle`
/// crate, where Go uses `subtle.ConstantTimeCompare`.
fn digests_equal(a: &[u8; 32], b: &[u8; 32]) -> bool {
    bool::from(a[..].ct_eq(&b[..]))
}

/// Answers a HELLO: OK, UNAUTHORIZED or UNSUPPORTED_VERSION, the token checked first. It compares SHA-256 digests,
/// so that the comparison does not show where the tokens differ (see [`digests_equal`]). Hashing the tokens first still
/// takes time that grows with each token's length.
pub fn authenticate(h: &Hello, token: &[u8]) -> Code {
    let (offered, want) = (sha256::digest(&h.token), sha256::digest(token));
    let same = digests_equal(&offered, &want);
    if !same || h.token.len() < MIN_TOKEN {
        return Code::UNAUTHORIZED;
    }
    if h.max_version < 1 {
        return Code::UNSUPPORTED_VERSION;
    }
    Code::OK
}

/// The server's answer to HELLO.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Welcome {
    pub status: Code,
    pub version: u16,
    pub max_body: u32,
    pub window: u16,
    pub window_bytes: u32,
}

/// Appends a WELCOME. With a status other than OK every later field is sent as zero.
pub fn append_welcome(dst: &mut Vec<u8>, w: &Welcome) {
    let w = if w.status != Code::OK {
        Welcome {
            status: w.status,
            ..Default::default()
        }
    } else {
        *w
    };
    dst.extend_from_slice(MAGIC);
    dst.push(w.status.0);
    dst.extend_from_slice(&w.version.to_le_bytes());
    dst.extend_from_slice(&w.max_body.to_le_bytes());
    dst.extend_from_slice(&w.window.to_le_bytes());
    dst.extend_from_slice(&w.window_bytes.to_le_bytes());
}

/// Decodes exactly [`WELCOME_SIZE`] bytes. Use [`Welcome::accept`] before sending anything else.
pub fn parse_welcome(b: &[u8]) -> Result<Welcome, Error> {
    if b.len() != WELCOME_SIZE || &b[..4] != MAGIC {
        return Err(Error::Protocol);
    }
    Ok(Welcome {
        status: Code(b[4]),
        version: u16_at(b, 5),
        max_body: u32_at(b, 7),
        window: u16_at(b, 11),
        window_bytes: u32_at(b, 13),
    })
}

impl Welcome {
    /// The client's validation of a WELCOME, given the highest version it offered. A version this module cannot
    /// decode is not accepted, whatever was offered. A refusal by the server (a known status with every later field
    /// zero) is an [`Error::Request`] with that status; any other unacceptable WELCOME is [`Error::Protocol`]. Both
    /// end the connection.
    pub fn accept(&self, offered: u16) -> Result<(), Error> {
        if self.status != Code::OK {
            if !self.status.known()
                || *self
                    != (Welcome {
                        status: self.status,
                        ..Default::default()
                    })
            {
                return Err(Error::Protocol);
            }
            return refuse(self.status);
        }
        if self.version < 1
            || self.version > offered
            || self.version > VERSION
            || self.max_body as usize != MAX_BODY
            || self.window < 1
            || (self.window_bytes as usize) < MAX_BODY + HEADER_SIZE
        {
            return Err(Error::Protocol);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! The Go package's tests (`wire/wire_test.go`), on the same vector file, and one more: every decision the Go
    //! decoders take on a few thousand damaged frames (`rust/testdata/wire-go.txt`).
    use super::*;
    use dostoevsky_format::gojson::{self, Value};

    const VECTORS: &str = include_str!("../../../../wire/testdata/vectors.json");
    const GO_DECISIONS: &str = include_str!("../../../testdata/wire-go.txt");

    fn unhex(h: &str) -> Vec<u8> {
        let h = h.trim_matches('-');
        (0..h.len() / 2)
            .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn member<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
        match v {
            Value::Object(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// A field that is a number, a decimal string, or absent (zero).
    fn num(v: &Value, key: &str) -> u64 {
        match member(v, key) {
            None | Some(Value::Null) => 0,
            Some(Value::Number(n)) | Some(Value::String(n)) => n.parse().unwrap(),
            Some(other) => panic!("not a number: {other:?}"),
        }
    }

    fn text<'a>(v: &'a Value, key: &str) -> &'a str {
        match member(v, key) {
            Some(Value::String(s)) => s,
            None | Some(Value::Null) => "",
            Some(other) => panic!("not text: {other:?}"),
        }
    }

    fn list<'a>(file: &'a Value, key: &str) -> &'a [Value] {
        match member(file, key) {
            Some(Value::Array(items)) if !items.is_empty() => items,
            _ => panic!("no vectors under {key}"),
        }
    }

    fn type_named(name: &str) -> Type {
        match name {
            "PUBLISH" => Type::PUBLISH,
            "READ" => Type::READ,
            "POSITION" => Type::POSITION,
            "COMMIT" => Type::COMMIT,
            "PING" => Type::PING,
            "PUBLISH_OK" => Type::PUBLISH_OK,
            "READ_OK" => Type::READ_OK,
            "POSITION_OK" => Type::POSITION_OK,
            "COMMIT_OK" => Type::COMMIT_OK,
            "PING_OK" => Type::PING_OK,
            "ERROR" => Type::ERROR,
            other => panic!("unknown type {other}"),
        }
    }

    #[test]
    fn limits_match_the_vector_file() {
        let file = gojson::parse(VECTORS.as_bytes()).unwrap();
        let limits = member(&file, "limits").unwrap();
        for (name, got) in [
            ("max_body", MAX_BODY),
            ("max_value", MAX_VALUE),
            ("max_key", MAX_KEY),
            ("max_id", MAX_ID),
            ("max_token", MAX_TOKEN),
            ("max_wait_ms", MAX_WAIT as usize),
        ] {
            assert_eq!(num(limits, name), got as u64, "{name}");
        }
    }

    /// Every valid vector is decoded to its listed fields and encoded back to the same bytes.
    #[test]
    fn valid_vectors() {
        let file = gojson::parse(VECTORS.as_bytes()).unwrap();
        for v in list(&file, "valid") {
            let (name, b) = (text(v, "name"), unhex(text(v, "hex")));
            let f = member(v, "fields").unwrap();
            let mut again = Vec::new();
            if name == "hello" {
                let h = read_hello(&mut &b[..]).unwrap();
                assert_eq!(
                    (h.max_version as u64, &h.token[..]),
                    (num(f, "max_version"), text(f, "token").as_bytes())
                );
                append_hello(&mut again, &h).unwrap();
            } else if name.starts_with("welcome") {
                let w = parse_welcome(&b).unwrap();
                let want = Welcome {
                    status: Code(num(f, "status") as u8),
                    version: num(f, "version") as u16,
                    max_body: num(f, "max_body") as u32,
                    window: num(f, "window") as u16,
                    window_bytes: num(f, "window_bytes") as u32,
                };
                assert_eq!(w, want, "{name}");
                append_welcome(&mut again, &w);
            } else {
                let (h, verdict) = parse_header(&b);
                assert_eq!(verdict, Ok(()), "{name}");
                assert_eq!(
                    (h.typ, h.request_id as u64),
                    (type_named(text(f, "type")), num(f, "request_id")),
                    "{name}"
                );
                let value = format!("{}{}", text(f, "value_hex"), text(f, "state_hex"));
                let offset = num(f, "offset") + num(f, "expected") + num(f, "next");
                let (key, id) = (unhex(text(f, "key_hex")), unhex(text(f, "id_hex")));
                if text(v, "sender") == "client" {
                    let (q, verdict) = decode_request(h, &b[HEADER_SIZE..]);
                    assert_eq!(verdict, Ok(()), "{name}");
                    assert_eq!(
                        (q.topic, q.group, q.partition as u64, q.offset),
                        (
                            text(f, "topic").as_bytes(),
                            text(f, "group").as_bytes(),
                            num(f, "partition"),
                            offset
                        ),
                        "{name}"
                    );
                    assert_eq!(
                        (q.wait_ms as u64, q.key, q.message_id, hex(q.value)),
                        (num(f, "wait_ms"), &key[..], &id[..], value),
                        "{name}"
                    );
                    append_request(&mut again, &q).unwrap();
                } else {
                    let (p, verdict) = decode_reply(h, &b[HEADER_SIZE..]);
                    assert_eq!(verdict, Ok(()), "{name}");
                    assert_eq!(
                        (p.partition as u64, p.offset, p.found, p.time as u64),
                        (
                            num(f, "partition"),
                            offset,
                            num(f, "found") == 1,
                            num(f, "time_unix_nanos")
                        ),
                        "{name}"
                    );
                    assert_eq!(
                        (p.key, p.message_id, hex(p.value), p.code.0 as u64),
                        (&key[..], &id[..], value, num(f, "code")),
                        "{name}"
                    );
                    append_reply(&mut again, &p).unwrap();
                }
            }
            assert_eq!(hex(&again), hex(&b), "{name} re-encoded");
        }
    }

    /// Every invalid vector gets the treatment the file names.
    #[test]
    fn invalid_vectors() {
        let token = b"example-token-not-a-secret";
        let file = gojson::parse(VECTORS.as_bytes()).unwrap();
        for v in list(&file, "invalid") {
            let (name, b, code) = (text(v, "name"), unhex(text(v, "hex")), Code(num(v, "code") as u8));
            match text(v, "scope") {
                "close-without-reply" => {
                    assert!(matches!(read_hello(&mut &b[..]), Err(HelloError::Protocol)), "{name}")
                }
                "welcome" => {
                    let h = read_hello(&mut &b[..]).unwrap();
                    // A short token is refused even if it is the configured one.
                    let configured: &[u8] = if h.token.len() < MIN_TOKEN { &h.token } else { token };
                    assert_eq!(authenticate(&h, configured), code, "{name}");
                }
                "connection" => {
                    let (h, mut verdict) = parse_header(&b);
                    if verdict.is_ok() {
                        verdict = decode_request(h, &b[HEADER_SIZE..]).1;
                    }
                    let want = if code == Code::LIMIT {
                        Error::TooLong
                    } else {
                        Error::Protocol
                    };
                    assert_eq!(verdict, Err(want), "{name}");
                }
                "request" => {
                    let (h, verdict) = parse_header(&b);
                    assert_eq!(verdict, Ok(()), "{name}");
                    let (q, verdict) = decode_request(h, &b[HEADER_SIZE..]);
                    assert_eq!(
                        (verdict, q.request_id),
                        (Err(Error::Request(code)), h.request_id),
                        "{name}"
                    );
                }
                "client-closes" => {
                    let (h, mut verdict) = parse_header(&b);
                    if verdict.is_ok() {
                        verdict = decode_reply(h, &b[HEADER_SIZE..]).1;
                    }
                    assert!(verdict.is_err(), "{name} accepted");
                }
                other => panic!("unknown scope {other}"),
            }
        }
    }

    #[test]
    fn authenticate_checks_the_token_before_the_version() {
        let token = b"0123456789abcdef";
        for (max_version, offered, want) in [
            (1, &token[..], Code::OK),
            (9, &token[..], Code::OK),
            (0, &token[..], Code::UNSUPPORTED_VERSION),
            (0, &b"0123456789abcdeX"[..], Code::UNAUTHORIZED),
            (1, &token[..15], Code::UNAUTHORIZED),
            (1, &b""[..], Code::UNAUTHORIZED),
        ] {
            assert_eq!(
                authenticate(
                    &Hello {
                        max_version,
                        token: offered.to_vec()
                    },
                    token
                ),
                want,
                "{max_version} {offered:?}"
            );
        }
    }

    #[test]
    fn a_client_accepts_only_a_usable_welcome() {
        let good = Welcome {
            status: Code::OK,
            version: 1,
            max_body: MAX_BODY as u32,
            window: 1,
            window_bytes: (MAX_BODY + HEADER_SIZE) as u32,
        };
        assert_eq!(good.accept(1), Ok(()));
        type Change = fn(&mut Welcome);
        let changes: [(&str, Change); 5] = [
            ("version zero", |w| w.version = 0),
            ("version not asked", |w| w.version = 2),
            ("another max_body", |w| w.max_body -= 1),
            ("no window", |w| w.window = 0),
            ("window_bytes small", |w| w.window_bytes -= 1),
        ];
        for (name, change) in changes {
            let mut w = good;
            change(&mut w);
            assert_eq!(w.accept(1), Err(Error::Protocol), "{name}");
        }
        for w in [
            Welcome {
                status: Code(99),
                ..Default::default()
            },
            Welcome {
                status: Code::UNAUTHORIZED,
                window: 1,
                ..Default::default()
            },
        ] {
            assert_eq!(w.accept(1), Err(Error::Protocol), "{w:?}");
        }
        // A later version is refused even by a caller that offered it.
        let later = Welcome {
            version: VERSION + 1,
            ..good
        };
        assert_eq!(later.accept(9), Err(Error::Protocol));
        assert_eq!(
            Welcome {
                status: Code::UNAUTHORIZED,
                ..Default::default()
            }
            .accept(1),
            Err(Error::Request(Code::UNAUTHORIZED))
        );
    }

    #[test]
    fn the_largest_legal_frames_fit() {
        let (long, key, id, value) = (
            vec![b'n'; MAX_NAME],
            vec![b'k'; MAX_KEY],
            vec![b'i'; MAX_ID],
            vec![0u8; MAX_VALUE],
        );
        for q in [
            Request {
                typ: Type::PUBLISH,
                request_id: 1,
                topic: &long,
                key: &key,
                message_id: &id,
                value: &value,
                ..Default::default()
            },
            Request {
                typ: Type::COMMIT,
                request_id: 1,
                topic: &long,
                group: &long,
                value: &value,
                ..Default::default()
            },
        ] {
            let mut b = Vec::new();
            assert_eq!(append_request(&mut b, &q), Ok(()));
            assert!(b.len() - HEADER_SIZE <= MAX_BODY, "type {:?}: {} bytes", q.typ, b.len());
        }
        let mut b = Vec::new();
        let p = Reply {
            typ: Type::READ_OK,
            request_id: 1,
            found: true,
            key: &key,
            message_id: &id,
            value: &value,
            ..Default::default()
        };
        assert_eq!(append_reply(&mut b, &p), Ok(()));
        assert!(b.len() - HEADER_SIZE <= MAX_BODY);
    }

    /// Replies outside the value table are rejected, and cannot be encoded either.
    #[test]
    fn reply_bounds() {
        let (long_key, long_state) = (vec![b'k'; MAX_KEY + 1], vec![0u8; MAX_VALUE + 1]);
        for (name, p) in [
            (
                "publish offset",
                Reply {
                    typ: Type::PUBLISH_OK,
                    request_id: 1,
                    offset: 1 << 63,
                    ..Default::default()
                },
            ),
            (
                "read offset",
                Reply {
                    typ: Type::READ_OK,
                    request_id: 1,
                    found: true,
                    offset: 1 << 63,
                    ..Default::default()
                },
            ),
            (
                "next position",
                Reply {
                    typ: Type::POSITION_OK,
                    request_id: 1,
                    offset: 1 << 63,
                    ..Default::default()
                },
            ),
            (
                "code zero",
                Reply {
                    typ: Type::ERROR,
                    request_id: 1,
                    ..Default::default()
                },
            ),
            (
                "unknown code",
                Reply {
                    typ: Type::ERROR,
                    request_id: 1,
                    code: Code(Code::UNSUPPORTED_VERSION.0 + 1),
                    ..Default::default()
                },
            ),
            (
                "key too long",
                Reply {
                    typ: Type::READ_OK,
                    request_id: 1,
                    found: true,
                    key: &long_key,
                    ..Default::default()
                },
            ),
            (
                "state too long",
                Reply {
                    typ: Type::POSITION_OK,
                    request_id: 1,
                    value: &long_state,
                    ..Default::default()
                },
            ),
        ] {
            let mut b = b"kept".to_vec();
            assert!(append_reply(&mut b, &p).is_err(), "{name}: encoded");
            assert_eq!(b, b"kept", "{name}: a refused reply must leave the buffer as it was");
        }
        let frame = |typ: Type, body: &[u8]| {
            decode_reply(
                Header {
                    length: body.len() as u32,
                    typ,
                    flags: 0,
                    request_id: 1,
                },
                body,
            )
            .1
        };
        let huge = [0, 0, 0, 0, 0, 0, 0, 0x80]; // 2^63
        for (name, verdict) in [
            (
                "publish offset",
                frame(Type::PUBLISH_OK, &[&[0, 0][..], &huge[..]].concat()),
            ),
            ("next position", frame(Type::POSITION_OK, &huge)),
            (
                "state too long",
                frame(Type::POSITION_OK, &vec![0u8; 8 + MAX_VALUE + 1]),
            ),
            ("code zero", frame(Type::ERROR, &[0])),
            ("unknown code", frame(Type::ERROR, &[13])),
        ] {
            assert_eq!(verdict, Err(Error::Protocol), "{name}");
        }
    }

    /// Appending after bytes already in the buffer leaves them alone.
    #[test]
    fn append_keeps_what_is_there() {
        let mut b = b"abc".to_vec();
        assert_eq!(
            append_request(
                &mut b,
                &Request {
                    typ: Type::PING,
                    request_id: 7,
                    ..Default::default()
                }
            ),
            Ok(())
        );
        assert_eq!(&b[..3], b"abc");
        let (h, verdict) = parse_header(&b[3..]);
        assert_eq!((verdict, h.request_id, h.length), (Ok(()), 7, 0));
    }

    #[test]
    fn code_names() {
        assert_eq!(
            [
                Code::OK.to_string(),
                Code::WRONG_OWNER.to_string(),
                Code::UNSUPPORTED_VERSION.to_string(),
                Code(13).to_string(),
                Code(255).to_string()
            ],
            ["ok", "wrong_owner", "unsupported_version", "code_13", "code_255"]
        );
    }

    fn verdict(v: Result<(), Error>) -> String {
        match v {
            Ok(()) => "ok".into(),
            Err(Error::Protocol) => "protocol".into(),
            Err(Error::TooLong) => "toolong".into(),
            Err(Error::Request(c)) => format!("refuse:{}", c.0),
        }
    }

    /// The Go decoders' verdict and decoded fields for every damaged frame, line for line. A frame either accepts
    /// must also encode back to the bytes received.
    #[test]
    fn every_decision_matches_the_go_codec() {
        let mut counts = std::collections::BTreeMap::new();
        for line in GO_DECISIONS.lines().filter(|l| !l.starts_with('#')) {
            let f: Vec<&str> = line.split(' ').collect();
            let b = unhex(f[1]);
            let x = |b: &[u8]| format!("-{}-", hex(b));
            let got = match f[0] {
                "H" => format!("H {} {}", f[1], verdict(parse_header(&b).1)),
                "Q" => {
                    let (h, ok) = parse_header(&b);
                    assert_eq!(ok, Ok(()), "{line}");
                    let (q, v) = decode_request(h, &b[HEADER_SIZE..]);
                    if v.is_ok() {
                        let mut again = Vec::new();
                        assert_eq!(append_request(&mut again, &q), Ok(()), "{line}");
                        assert_eq!(again, b, "{line}");
                    }
                    format!(
                        "Q {} {} {} {} {} {} {} {} {} {} {} {}",
                        f[1],
                        verdict(v),
                        q.typ.0,
                        q.request_id,
                        x(q.topic),
                        q.partition,
                        q.offset,
                        q.wait_ms,
                        x(q.group),
                        x(q.key),
                        x(q.message_id),
                        x(q.value)
                    )
                }
                "P" => {
                    let (h, ok) = parse_header(&b);
                    assert_eq!(ok, Ok(()), "{line}");
                    let (p, v) = decode_reply(h, &b[HEADER_SIZE..]);
                    if v.is_ok() {
                        let mut again = Vec::new();
                        assert_eq!(append_reply(&mut again, &p), Ok(()), "{line}");
                        assert_eq!(again, b, "{line}");
                    }
                    format!(
                        "P {} {} {} {} {} {} {} {} {} {} {} {}",
                        f[1],
                        verdict(v),
                        p.typ.0,
                        p.request_id,
                        p.partition,
                        p.offset,
                        p.found,
                        p.time,
                        x(p.key),
                        x(p.message_id),
                        x(p.value),
                        p.code.0
                    )
                }
                "L" => {
                    let mut reader = &b[..];
                    let (v, hello) = match read_hello(&mut reader) {
                        Ok(h) => ("ok", h),
                        Err(HelloError::Protocol) => ("protocol", Hello::default()),
                        Err(HelloError::Eof) => ("eof", Hello::default()),
                        Err(HelloError::UnexpectedEof) => ("unexpected", Hello::default()),
                        Err(HelloError::Io(e)) => panic!("{e}"),
                    };
                    format!(
                        "L {} {} {} {} {}",
                        f[1],
                        v,
                        hello.max_version,
                        x(&hello.token),
                        b.len() - reader.len()
                    )
                }
                "W" => {
                    let parsed = parse_welcome(&b);
                    let w = parsed.unwrap_or_default();
                    let mut again = Vec::new();
                    append_welcome(&mut again, &w);
                    format!(
                        "W {} {} {} {} {} {} {} {} {}",
                        f[1],
                        verdict(parsed.map(|_| ())),
                        w.status.0,
                        w.version,
                        w.max_body,
                        w.window,
                        w.window_bytes,
                        verdict(w.accept(1)),
                        x(&again)
                    )
                }
                other => panic!("unknown decoder {other}"),
            };
            assert_eq!(got, line);
            *counts.entry(format!("{} {}", f[0], f[2])).or_insert(0) += 1;
        }
        assert!(
            counts["Q ok"] > 200
                && counts["P ok"] > 200
                && counts["Q protocol"] > 1000
                && counts["H toolong"] > 500
                && counts["L ok"] > 100,
            "{counts:?}"
        );
    }
}
