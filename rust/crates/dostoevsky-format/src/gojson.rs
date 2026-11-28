//! JSON as Go's `encoding/json` reads and writes it.
//!
//! The Go engine writes its legacy journal, several files and its driver replies with `encoding/json`, so that
//! encoder's output is part of the format: key order, HTML escaping, the escapes it chooses and how it treats
//! invalid UTF-8. The format is defined as the output of Go 1.27.0.
//!
//! Writing. [`write_string`] is Go's `appendString`. Invalid UTF-8 is written as the three raw bytes EF BF BD, one
//! replacement per invalid byte ([`InvalidUtf8::Raw`], the Go 1.27.0 form). Earlier releases wrote the six characters
//! `\ufffd` instead; [`InvalidUtf8::Escaped`] writes that form, and exists so the writer can be checked against the
//! vectors of an older toolchain. The reader accepts both.
//!
//! Reading. [`parse_prefix`] accepts what Go's decoder accepts and `serde_json` would refuse: invalid UTF-8 inside
//! strings (replaced by U+FFFD, one per invalid byte), unpaired surrogate escapes (replaced the same way), duplicate
//! keys (all kept, in order; the caller applies Go's "last one wins"), and nesting up to Go's limit of 10,000.
//!
//! Not here yet, each added in the row that first needs it: floating-point numbers in Go's format, and times in
//! RFC 3339 form.

use std::fmt;

/// Go refuses input nested deeper than this.
pub const MAX_DEPTH: usize = 10_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The literal as it appeared (or as it is to be written). Kept as text so that no reader rounds it.
    Number(String),
    String(String),
    Array(Vec<Value>),
    /// Members in the order given. Go writes struct fields in declaration order and map keys sorted by bytes; the
    /// caller builds the order (see [`Value::map`]). After parsing, duplicate keys are all present.
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn int(n: i64) -> Value {
        Value::Number(n.to_string())
    }

    pub fn text(s: impl Into<String>) -> Value {
        Value::String(s.into())
    }

    /// A Go `[]byte`: base64, or `null` for a nil slice.
    pub fn bytes(b: Option<&[u8]>) -> Value {
        match b {
            Some(b) => Value::String(crate::base64::encode(b)),
            None => Value::Null,
        }
    }

    /// A Go map: keys sorted by their bytes.
    pub fn map(mut members: Vec<(String, Value)>) -> Value {
        members.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        Value::Object(members)
    }
}

fn take_children(v: &mut Value, into: &mut Vec<Value>) {
    match v {
        Value::Array(items) => into.append(items),
        Value::Object(members) => into.extend(members.drain(..).map(|(_, v)| v)),
        _ => {}
    }
}

/// Dropping a deeply nested value must not recurse once per level.
impl Drop for Value {
    fn drop(&mut self) {
        if !matches!(self, Value::Array(_) | Value::Object(_)) {
            return;
        }
        let mut rest = Vec::new();
        take_children(self, &mut rest);
        while let Some(mut v) = rest.pop() {
            take_children(&mut v, &mut rest);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidUtf8 {
    /// EF BF BD: the Go 1.27.0 form, and the format.
    Raw,
    /// `\ufffd`: the form of earlier releases.
    Escaped,
}

#[derive(Debug, Clone, Copy)]
pub struct WriteOptions {
    /// Go's default: `<`, `>` and `&` are written as `\u003c`, `\u003e` and `\u0026`.
    pub escape_html: bool,
    pub invalid_utf8: InvalidUtf8,
}

impl Default for WriteOptions {
    fn default() -> Self {
        WriteOptions {
            escape_html: true,
            invalid_utf8: InvalidUtf8::Raw,
        }
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Appends a JSON string for `s`, which is a Go string: bytes that are usually, not always, UTF-8.
pub fn write_string(out: &mut Vec<u8>, s: &[u8], opts: WriteOptions) {
    out.push(b'"');
    let mut start = 0;
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        if b < 0x80 {
            let safe = b >= 0x20 && b != b'"' && b != b'\\' && !(opts.escape_html && matches!(b, b'<' | b'>' | b'&'));
            if safe {
                i += 1;
                continue;
            }
            out.extend_from_slice(&s[start..i]);
            match b {
                b'\\' | b'"' => out.extend_from_slice(&[b'\\', b]),
                0x08 => out.extend_from_slice(b"\\b"),
                0x0c => out.extend_from_slice(b"\\f"),
                b'\n' => out.extend_from_slice(b"\\n"),
                b'\r' => out.extend_from_slice(b"\\r"),
                b'\t' => out.extend_from_slice(b"\\t"),
                _ => out.extend_from_slice(&[b'\\', b'u', b'0', b'0', HEX[(b >> 4) as usize], HEX[(b & 15) as usize]]),
            }
            i += 1;
            start = i;
            continue;
        }
        match decode_rune(&s[i..]) {
            None => {
                out.extend_from_slice(&s[start..i]);
                match opts.invalid_utf8 {
                    InvalidUtf8::Raw => out.extend_from_slice("\u{fffd}".as_bytes()),
                    InvalidUtf8::Escaped => out.extend_from_slice(b"\\ufffd"),
                }
                i += 1;
                start = i;
            }
            // U+2028 and U+2029 are valid JSON but break JSONP; Go always escapes them.
            Some((c, n)) if c == '\u{2028}' || c == '\u{2029}' => {
                out.extend_from_slice(&s[start..i]);
                out.extend_from_slice(b"\\u202");
                out.push(HEX[(c as u32 & 15) as usize]);
                i += n;
                start = i;
            }
            Some((_, n)) => i += n,
        }
    }
    out.extend_from_slice(&s[start..]);
    out.push(b'"');
}

/// The rune at the start of `s` and its length, or None where Go's `utf8.DecodeRune` reports an error of width 1.
fn decode_rune(s: &[u8]) -> Option<(char, usize)> {
    let width = match s[0] {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let c = std::str::from_utf8(s.get(..width)?).ok()?.chars().next()?;
    Some((c, width))
}

/// Replaces each invalid byte by U+FFFD, as Go does when it converts JSON text to a string.
fn to_string_like_go(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                return out;
            }
            Err(e) => {
                let good = e.valid_up_to();
                out.push_str(std::str::from_utf8(&rest[..good]).expect("checked prefix"));
                out.push('\u{fffd}');
                rest = &rest[good + 1..];
            }
        }
    }
}

enum Open<'a> {
    Array(std::slice::Iter<'a, Value>, bool),
    Object(std::slice::Iter<'a, (String, Value)>, bool),
}

/// Appends `v` in Go's compact form (no spaces, no trailing newline).
///
/// It keeps its own stack: a value nested to Go's limit must not overflow the thread's.
pub fn write_value(out: &mut Vec<u8>, v: &Value, opts: WriteOptions) {
    let mut open: Vec<Open> = Vec::new();
    let mut next = Some(v);
    loop {
        if let Some(v) = next.take() {
            match v {
                Value::Null => out.extend_from_slice(b"null"),
                Value::Bool(true) => out.extend_from_slice(b"true"),
                Value::Bool(false) => out.extend_from_slice(b"false"),
                Value::Number(n) => out.extend_from_slice(n.as_bytes()),
                Value::String(s) => write_string(out, s.as_bytes(), opts),
                Value::Array(items) => {
                    out.push(b'[');
                    open.push(Open::Array(items.iter(), true));
                }
                Value::Object(members) => {
                    out.push(b'{');
                    open.push(Open::Object(members.iter(), true));
                }
            }
        }
        match open.last_mut() {
            None => return,
            Some(Open::Array(items, first)) => match items.next() {
                Some(item) => {
                    if !std::mem::replace(first, false) {
                        out.push(b',');
                    }
                    next = Some(item);
                }
                None => {
                    out.push(b']');
                    open.pop();
                }
            },
            Some(Open::Object(members, first)) => match members.next() {
                Some((key, value)) => {
                    if !std::mem::replace(first, false) {
                        out.push(b',');
                    }
                    write_string(out, key.as_bytes(), opts);
                    out.push(b':');
                    next = Some(value);
                }
                None => {
                    out.push(b'}');
                    open.pop();
                }
            },
        }
    }
}

/// One value followed by a line feed, as Go's `json.Encoder.Encode`.
pub fn encode_line(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_value(&mut out, v, WriteOptions::default());
    out.push(b'\n');
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The input ended inside a value. With more input it may still be valid.
    Incomplete,
    Syntax {
        offset: usize,
        message: &'static str,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Incomplete => write!(f, "unexpected end of JSON input"),
            ParseError::Syntax { offset, message } => {
                write!(f, "invalid JSON at byte {offset}: {message}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

struct Parser<'a> {
    input: &'a [u8],
    at: usize,
    at_eof: bool,
}

type Parsed<T> = Result<T, ParseError>;

impl Parser<'_> {
    fn bad<T>(&self, message: &'static str) -> Parsed<T> {
        Err(ParseError::Syntax {
            offset: self.at,
            message,
        })
    }

    fn skip_space(&mut self) {
        while matches!(self.input.get(self.at), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.at += 1;
        }
    }

    fn literal(&mut self, word: &'static [u8], v: Value) -> Parsed<Value> {
        for (k, &c) in word.iter().enumerate() {
            match self.input.get(self.at + k) {
                None => return Err(ParseError::Incomplete),
                Some(&got) if got != c => {
                    self.at += k;
                    return self.bad("invalid character in literal");
                }
                Some(_) => {}
            }
        }
        self.at += word.len();
        Ok(v)
    }

    /// The key and colon of the next object member.
    fn key(&mut self) -> Parsed<String> {
        self.skip_space();
        match self.input.get(self.at) {
            None => return Err(ParseError::Incomplete),
            Some(b'"') => {}
            Some(_) => return self.bad("looking for beginning of object key string"),
        }
        let key = self.string()?;
        self.skip_space();
        match self.input.get(self.at) {
            None => Err(ParseError::Incomplete),
            Some(b':') => {
                self.at += 1;
                Ok(key)
            }
            Some(_) => self.bad("invalid character after object key"),
        }
    }

    /// One value. Containers are kept on a stack of our own: input nested to Go's limit must not overflow the
    /// thread's.
    fn value(&mut self) -> Parsed<Value> {
        enum Open {
            Array(Vec<Value>),
            Object(Vec<(String, Value)>, String),
        }
        let mut open: Vec<Open> = Vec::new();
        loop {
            self.skip_space();
            let Some(&c) = self.input.get(self.at) else {
                return Err(ParseError::Incomplete);
            };
            let mut done = match c {
                b'{' | b'[' => {
                    if open.len() >= MAX_DEPTH {
                        return self.bad("exceeded max depth");
                    }
                    self.at += 1;
                    self.skip_space();
                    let close = if c == b'{' { b'}' } else { b']' };
                    match self.input.get(self.at) {
                        None => return Err(ParseError::Incomplete),
                        Some(&x) if x == close => {
                            self.at += 1;
                            if c == b'{' {
                                Value::Object(Vec::new())
                            } else {
                                Value::Array(Vec::new())
                            }
                        }
                        Some(_) => {
                            if c == b'{' {
                                let key = self.key()?;
                                open.push(Open::Object(Vec::new(), key));
                            } else {
                                open.push(Open::Array(Vec::new()));
                            }
                            continue;
                        }
                    }
                }
                b'"' => Value::String(self.string()?),
                b't' => self.literal(b"true", Value::Bool(true))?,
                b'f' => self.literal(b"false", Value::Bool(false))?,
                b'n' => self.literal(b"null", Value::Null)?,
                b'-' | b'0'..=b'9' => self.number()?,
                _ => return self.bad("invalid character looking for beginning of value"),
            };
            // Hand the finished value to the container it belongs to, closing containers as they end.
            loop {
                let Some(top) = open.last_mut() else {
                    return Ok(done);
                };
                self.skip_space();
                let Some(&c) = self.input.get(self.at) else {
                    return Err(ParseError::Incomplete);
                };
                match top {
                    Open::Array(items) => {
                        items.push(done);
                        match c {
                            b',' => {
                                self.at += 1;
                                break;
                            }
                            b']' => self.at += 1,
                            _ => return self.bad("invalid character after array element"),
                        }
                    }
                    Open::Object(members, key) => {
                        members.push((std::mem::take(key), done));
                        match c {
                            b',' => {
                                self.at += 1;
                                *key = self.key()?;
                                break;
                            }
                            b'}' => self.at += 1,
                            _ => return self.bad("invalid character after object key:value pair"),
                        }
                    }
                }
                done = match open.pop() {
                    Some(Open::Array(items)) => Value::Array(items),
                    Some(Open::Object(members, _)) => Value::Object(members),
                    None => unreachable!("a container was open"),
                };
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while matches!(self.input.get(self.at), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Parsed<Value> {
        let start = self.at;
        let end = |p: &Self| {
            if p.at == p.input.len() && !p.at_eof {
                Err(ParseError::Incomplete)
            } else {
                Ok(())
            }
        };
        if self.input[self.at] == b'-' {
            self.at += 1;
        }
        match self.input.get(self.at) {
            None => return Err(ParseError::Incomplete),
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            Some(_) => return self.bad("invalid character in numeric literal"),
        }
        if self.input.get(self.at) == Some(&b'.') {
            self.at += 1;
            if self.digits() == 0 {
                end(self)?;
                return self.bad("invalid character after decimal point in numeric literal");
            }
        }
        if matches!(self.input.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.input.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                end(self)?;
                return self.bad("invalid character in exponent of numeric literal");
            }
        }
        end(self)?;
        Ok(Value::Number(
            String::from_utf8(self.input[start..self.at].to_vec()).expect("number literals are ASCII"),
        ))
    }

    fn hex4(&mut self) -> Parsed<u32> {
        let Some(h) = self.input.get(self.at..self.at + 4) else {
            return Err(ParseError::Incomplete);
        };
        let mut v = 0u32;
        for &c in h {
            let d = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => return self.bad("invalid character in \\u hexadecimal character escape"),
            };
            v = v << 4 | d as u32;
        }
        self.at += 4;
        Ok(v)
    }

    /// The string whose opening quote is at `self.at`.
    fn string(&mut self) -> Parsed<String> {
        self.at += 1;
        let mut raw: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.input.get(self.at) else {
                return Err(ParseError::Incomplete);
            };
            match c {
                b'"' => {
                    self.at += 1;
                    return Ok(to_string_like_go(&raw));
                }
                0x00..=0x1f => return self.bad("invalid character in string literal"),
                b'\\' => {
                    self.at += 1;
                    let Some(&e) = self.input.get(self.at) else {
                        return Err(ParseError::Incomplete);
                    };
                    self.at += 1;
                    match e {
                        b'"' | b'\\' | b'/' => raw.push(e),
                        b'b' => raw.push(0x08),
                        b'f' => raw.push(0x0c),
                        b'n' => raw.push(b'\n'),
                        b'r' => raw.push(b'\r'),
                        b't' => raw.push(b'\t'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xd800..0xe000).contains(&code) {
                                // A high surrogate followed by an escaped low surrogate is one character. Anything
                                // else is replaced, and what follows is read again on its own.
                                let mark = self.at;
                                let mut paired = false;
                                if code < 0xdc00 && self.input.get(self.at..self.at + 2) == Some(b"\\u") {
                                    self.at += 2;
                                    match self.hex4() {
                                        Ok(low) if (0xdc00..0xe000).contains(&low) => {
                                            code = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
                                            paired = true;
                                        }
                                        Ok(_) => {}
                                        Err(e) => return Err(e),
                                    }
                                }
                                if !paired {
                                    self.at = mark;
                                    code = 0xfffd;
                                }
                            }
                            let c = char::from_u32(code).unwrap_or('\u{fffd}');
                            raw.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        _ => {
                            self.at -= 1;
                            return self.bad("invalid character in string escape code");
                        }
                    }
                }
                _ => {
                    raw.push(c);
                    self.at += 1;
                }
            }
        }
    }
}

/// Parses the first value in `input` and returns it with the number of bytes it used.
///
/// `at_eof` says that no more input can follow. Without it a number that runs to the end of `input` is
/// [`ParseError::Incomplete`], because more digits may follow.
pub fn parse_prefix(input: &[u8], at_eof: bool) -> Result<(Value, usize), ParseError> {
    let mut p = Parser { input, at: 0, at_eof };
    let v = p.value()?;
    Ok((v, p.at))
}

/// Parses `input` as exactly one value, as Go's `json.Unmarshal`.
pub fn parse(input: &[u8]) -> Result<Value, ParseError> {
    let mut p = Parser {
        input,
        at: 0,
        at_eof: true,
    };
    let v = p.value()?;
    p.skip_space();
    if p.at != input.len() {
        return p.bad("invalid character after top-level value");
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(s: &[u8], opts: WriteOptions) -> Vec<u8> {
        let mut out = Vec::new();
        write_string(&mut out, s, opts);
        out
    }

    /// Vectors written by the Go encoder (see `rust/testdata/README.md`): one case per line, the input and the
    /// output of `json.Marshal` in hex, with and without HTML escaping.
    #[test]
    fn strings_match_the_go_encoder() {
        let vectors = include_str!("../../../testdata/gojson-strings.txt");
        let unhex = |h: &str| {
            (0..h.len() / 2)
                .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap())
                .collect::<Vec<u8>>()
        };
        let mut checked = 0;
        let mut form = InvalidUtf8::Raw;
        for line in vectors.lines() {
            if let Some(rest) = line.strip_prefix("# invalid-utf8: ") {
                form = if rest == "escaped" {
                    InvalidUtf8::Escaped
                } else {
                    InvalidUtf8::Raw
                };
                continue;
            }
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split(' ').collect();
            let (input, html, plain) = (unhex(fields[0].trim_matches('-')), unhex(fields[1]), unhex(fields[2]));
            assert_eq!(
                written(
                    &input,
                    WriteOptions {
                        escape_html: true,
                        invalid_utf8: form
                    }
                ),
                html,
                "input {}",
                fields[0]
            );
            assert_eq!(
                written(
                    &input,
                    WriteOptions {
                        escape_html: false,
                        invalid_utf8: form
                    }
                ),
                plain,
                "input {}",
                fields[0]
            );
            // What Go wrote, read back, is the input with each invalid byte replaced.
            assert_eq!(
                parse(&html).unwrap(),
                Value::String(to_string_like_go(&input)),
                "input {}",
                fields[0]
            );
            checked += 1;
        }
        assert!(checked > 1000, "only {checked} vectors");
    }

    /// Vectors from the Go decoder: what it refuses, what it accepts, and the string it decodes.
    #[test]
    fn reads_and_refuses_exactly_what_the_go_decoder_does() {
        let vectors = include_str!("../../../testdata/gojson-reader.txt");
        let unhex = |h: &str| {
            (0..h.len() / 2)
                .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap())
                .collect::<Vec<u8>>()
        };
        let (mut strings, mut refused, mut other) = (0, 0, 0);
        for line in vectors.lines().filter(|l| !l.starts_with('#')) {
            let (input, want) = line.split_once(' ').unwrap();
            let got = parse(&unhex(input.trim_matches('-')));
            let short = &input[..input.len().min(60)];
            match want {
                "invalid" => {
                    assert!(got.is_err(), "Go refuses {short}");
                    refused += 1;
                }
                "valid" => {
                    assert!(got.is_ok(), "Go accepts {short}");
                    other += 1;
                }
                text => {
                    assert_eq!(
                        got,
                        Ok(Value::String(String::from_utf8(unhex(text.trim_matches('-'))).unwrap())),
                        "input {short}"
                    );
                    strings += 1;
                }
            }
        }
        assert!(
            strings > 300 && refused > 300 && other >= 20,
            "{strings} strings, {refused} refused, {other} other"
        );
    }

    #[test]
    fn invalid_utf8_is_written_raw_and_both_forms_read_the_same() {
        let input = b"a\xffb\xe2\x82c";
        let raw = written(input, WriteOptions::default());
        assert_eq!(raw, b"\"a\xef\xbf\xbdb\xef\xbf\xbd\xef\xbf\xbdc\"");
        let escaped = written(
            input,
            WriteOptions {
                invalid_utf8: InvalidUtf8::Escaped,
                ..Default::default()
            },
        );
        assert_eq!(escaped, b"\"a\\ufffdb\\ufffd\\ufffdc\"");
        assert_eq!(parse(&raw).unwrap(), parse(&escaped).unwrap());
        // History may hold the invalid bytes themselves: one replacement per byte, as Go.
        assert_eq!(
            parse(b"\"a\xffb\xe2\x82c\"").unwrap(),
            Value::text("a\u{fffd}b\u{fffd}\u{fffd}c")
        );
    }

    #[test]
    fn reads_what_go_reads() {
        let v = parse(
            b" {\"a\":1,\"a\":2,\"b\":[true,false,null,-0.5e+3,\"\\ud83d\\ude00 \\ud800 \\udc00x \\u00e9\\/\"]}\n",
        )
        .unwrap();
        let Value::Object(m) = &v else { panic!() };
        assert_eq!(m[0], ("a".into(), Value::Number("1".into())));
        assert_eq!(m[1], ("a".into(), Value::Number("2".into())));
        let Value::Array(items) = &m[2].1 else { panic!() };
        assert_eq!(items[3], Value::Number("-0.5e+3".into()));
        assert_eq!(items[4], Value::text("\u{1f600} \u{fffd} \u{fffd}x \u{e9}/"));
        let deep = |n: usize| [vec![b'['; n], vec![b']'; n]].concat();
        let deepest = parse(&deep(MAX_DEPTH)).unwrap();
        let mut back = Vec::new();
        write_value(&mut back, &deepest, WriteOptions::default());
        assert_eq!(back, deep(MAX_DEPTH));
        let mut inner = &deepest;
        let mut levels = 0;
        while let Value::Array(items) = inner {
            levels += 1;
            match items.first() {
                Some(v) => inner = v,
                None => break,
            }
        }
        assert_eq!(levels, MAX_DEPTH);
        assert!(matches!(parse(&deep(MAX_DEPTH + 1)), Err(ParseError::Syntax { .. })));
    }

    #[test]
    fn refuses_what_go_refuses() {
        for bad in [
            "01",
            "1.",
            "1e",
            "-",
            "+1",
            ".5",
            "tru",
            "nul",
            "[1,]",
            "{\"a\":1,}",
            "{a:1}",
            "\"\t\"",
            "\"\\x\"",
            "\"\\u12g4\"",
            "1 2",
            "",
            "[",
            "\"abc",
        ] {
            assert!(parse(bad.as_bytes()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_stream_waits_for_the_rest() {
        assert_eq!(parse_prefix(b"{\"a\":", false), Err(ParseError::Incomplete));
        assert_eq!(parse_prefix(b"12", false), Err(ParseError::Incomplete));
        assert_eq!(parse_prefix(b"12", true), Ok((Value::Number("12".into()), 2)));
        assert_eq!(parse_prefix(b"12 {", false), Ok((Value::Number("12".into()), 2)));
        assert_eq!(parse_prefix(b"\"\\ud83d", false), Err(ParseError::Incomplete));
        assert_eq!(parse_prefix(b"{}{}", false), Ok((Value::Object(vec![]), 2)));
    }

    #[test]
    fn writes_values_as_go_does() {
        let v = Value::Object(vec![
            ("ok".into(), Value::Bool(true)),
            (
                "result".into(),
                Value::map(vec![
                    ("truncated_bytes".into(), Value::int(0)),
                    ("truncated_at".into(), Value::int(-1)),
                    ("state".into(), Value::bytes(None)),
                ]),
            ),
            ("value".into(), Value::bytes(Some(b"<a&b>"))),
            ("text".into(), Value::text("<a&b>\u{2028}")),
            ("list".into(), Value::Array(vec![])),
        ]);
        assert_eq!(
            String::from_utf8(encode_line(&v)).unwrap(),
            "{\"ok\":true,\"result\":{\"state\":null,\"truncated_at\":-1,\"truncated_bytes\":0},\"value\":\"PGEmYj4=\",\"text\":\"\\u003ca\\u0026b\\u003e\\u2028\",\"list\":[]}\n"
        );
    }
}
