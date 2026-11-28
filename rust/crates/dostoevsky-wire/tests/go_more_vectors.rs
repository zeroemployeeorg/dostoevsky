//! Checks the wire codec against the Go package on large inputs and on what the encoders refuse, row for row. The rows
//! and their format come from `rust/testdata/wire_more_vectors.go`.

use dostoevsky_format::crc32c;
use dostoevsky_wire::wire::*;
use std::collections::BTreeMap;

/// Runs of bytes: `hex*count` joined by commas, `-` for none.
fn bytes(spec: &str) -> Vec<u8> {
    let mut out = Vec::new();
    if spec == "-" {
        return out;
    }
    for run in spec.split(',') {
        let (digits, count) = run.split_once('*').unwrap();
        let unit: Vec<u8> = (0..digits.len() / 2)
            .map(|i| u8::from_str_radix(&digits[2 * i..2 * i + 2], 16).unwrap())
            .collect();
        let count = count.parse::<usize>().unwrap();
        match unit[..] {
            [one] => out.resize(out.len() + count, one),
            _ => (0..count).for_each(|_| out.extend_from_slice(&unit)),
        }
    }
    out
}

/// Up to 48 bytes in hex between dashes; more as the length and CRC-32C.
fn show(b: &[u8]) -> String {
    if b.len() <= 48 {
        format!("-{}-", b.iter().map(|x| format!("{x:02x}")).collect::<String>())
    } else {
        format!("{}:{:08x}", b.len(), crc32c::checksum(b))
    }
}

fn verdict(v: Result<(), Error>) -> String {
    match v {
        Ok(()) => "ok".into(),
        Err(Error::Protocol) => "protocol".into(),
        Err(Error::TooLong) => "toolong".into(),
        Err(Error::Request(c)) => format!("refuse:{}", c.0),
    }
}

/// What an encoder left: the frame after the three bytes the destination held, or whether a refusal kept them.
fn appended(dst: &[u8], v: &Result<(), Error>) -> String {
    match v {
        Ok(()) => {
            assert_eq!(&dst[..3], b"abc");
            show(&dst[3..])
        }
        Err(_) if dst == b"abc" => "kept".into(),
        Err(_) => "changed".into(),
    }
}

#[test]
fn large_inputs_and_encoder_refusals_match_the_go_codec() {
    let rows = include_str!("../../../testdata/wire-more-go.txt");
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let (mut megabyte_inputs, mut megabyte_frames) = (0, 0);
    for row in rows.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = row.split(' ').collect();
        let n = |i: usize| f[i].parse::<u64>().unwrap();
        let got = match f[0] {
            "BH" | "BQ" | "BP" => {
                let b = bytes(f[1]);
                megabyte_inputs += usize::from(b.len() > 1 << 20);
                let (h, header) = parse_header(&b);
                match (f[0], header) {
                    ("BH", header) => format!("BH {} {}", f[1], verdict(header)),
                    (_, Err(e)) => panic!("{row}: the header is refused: {e}"),
                    ("BQ", Ok(())) => {
                        let (q, v) = decode_request(h, &b[HEADER_SIZE..]);
                        if v.is_ok() {
                            let mut again = Vec::new();
                            assert_eq!(append_request(&mut again, &q), Ok(()), "{row}");
                            assert!(again == b, "{row}: does not encode back to the bytes received");
                        }
                        format!(
                            "BQ {} {} {} {} {} {} {} {} {} {} {} {}",
                            f[1],
                            verdict(v),
                            q.typ.0,
                            q.request_id,
                            show(q.topic),
                            q.partition,
                            q.offset,
                            q.wait_ms,
                            show(q.group),
                            show(q.key),
                            show(q.message_id),
                            show(q.value)
                        )
                    }
                    (_, Ok(())) => {
                        let (p, v) = decode_reply(h, &b[HEADER_SIZE..]);
                        if v.is_ok() {
                            let mut again = Vec::new();
                            assert_eq!(append_reply(&mut again, &p), Ok(()), "{row}");
                            assert!(again == b, "{row}: does not encode back to the bytes received");
                        }
                        format!(
                            "BP {} {} {} {} {} {} {} {} {} {} {} {}",
                            f[1],
                            verdict(v),
                            p.typ.0,
                            p.request_id,
                            p.partition,
                            p.offset,
                            p.found,
                            p.time,
                            show(p.key),
                            show(p.message_id),
                            show(p.value),
                            p.code.0
                        )
                    }
                }
            }
            "BL" => {
                let b = bytes(f[1]);
                let mut reader = &b[..];
                let (v, hello) = match read_hello(&mut reader) {
                    Ok(h) => ("ok", h),
                    Err(HelloError::Protocol) => ("protocol", Hello::default()),
                    Err(HelloError::Eof) => ("eof", Hello::default()),
                    Err(HelloError::UnexpectedEof) => ("unexpected", Hello::default()),
                    Err(HelloError::Io(e)) => panic!("{e}"),
                };
                format!(
                    "BL {} {} {} {} {}",
                    f[1],
                    v,
                    hello.max_version,
                    show(&hello.token),
                    b.len() - reader.len()
                )
            }
            "EQ" => {
                let (topic, group, key, id, value) = (bytes(f[3]), bytes(f[7]), bytes(f[8]), bytes(f[9]), bytes(f[10]));
                let q = Request {
                    typ: Type(n(1) as u8),
                    request_id: n(2) as u32,
                    topic: &topic,
                    partition: n(4) as u16,
                    offset: n(5),
                    wait_ms: n(6) as u16,
                    group: &group,
                    key: &key,
                    message_id: &id,
                    value: &value,
                };
                let mut dst = b"abc".to_vec();
                let v = append_request(&mut dst, &q);
                megabyte_frames += usize::from(v.is_ok() && dst.len() > 1 << 20);
                let tail = appended(&dst, &v);
                format!("{} {} {}", f[..11].join(" "), verdict(v), tail)
            }
            "EP" => {
                let (key, id, value) = (bytes(f[7]), bytes(f[8]), bytes(f[9]));
                let p = Reply {
                    typ: Type(n(1) as u8),
                    request_id: n(2) as u32,
                    partition: n(3) as u16,
                    offset: n(4),
                    found: f[5] == "true",
                    time: f[6].parse().unwrap(),
                    key: &key,
                    message_id: &id,
                    value: &value,
                    code: Code(n(10) as u8),
                };
                let mut dst = b"abc".to_vec();
                let v = append_reply(&mut dst, &p);
                megabyte_frames += usize::from(v.is_ok() && dst.len() > 1 << 20);
                let tail = appended(&dst, &v);
                format!("{} {} {}", f[..11].join(" "), verdict(v), tail)
            }
            "EH" => {
                let hello = Hello {
                    max_version: n(1) as u16,
                    token: bytes(f[2]),
                };
                let mut dst = b"abc".to_vec();
                let v = append_hello(&mut dst, &hello);
                let tail = appended(&dst, &v);
                format!("{} {} {}", f[..3].join(" "), verdict(v), tail)
            }
            other => panic!("unknown row {other}"),
        };
        assert!(got == row, "Go:   {row}\nRust: {got}");
        let verdict_at = match f[0] {
            "EQ" | "EP" => 11,
            "EH" => 3,
            _ => 2,
        };
        *counts.entry(format!("{} {}", f[0], f[verdict_at])).or_insert(0) += 1;
    }
    // The rows must still hold what they are for: every verdict of every decoder and encoder, and megabyte inputs.
    for (kind, least) in [
        ("BH toolong", 30),
        ("BH protocol", 150),
        ("BQ ok", 30),
        ("BQ protocol", 600),
        ("BQ refuse:1", 300),
        ("BQ refuse:6", 150),
        ("BP ok", 15),
        ("BP protocol", 1000),
        ("BL ok", 10),
        ("BL protocol", 30),
        ("BL unexpected", 4),
        ("EQ ok", 150),
        ("EQ protocol", 1000),
        ("EQ refuse:1", 1500),
        ("EQ refuse:6", 200),
        ("EP ok", 200),
        ("EP protocol", 2000),
        ("EH ok", 15),
        ("EH protocol", 12),
    ] {
        assert!(counts.get(kind).copied().unwrap_or(0) >= least, "{kind}: {counts:?}");
    }
    assert!(
        megabyte_inputs > 300 && megabyte_frames >= 6,
        "{megabyte_inputs} inputs and {megabyte_frames} encoded frames above a megabyte"
    );
}
