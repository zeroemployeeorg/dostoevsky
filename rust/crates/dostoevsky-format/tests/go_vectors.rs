//! The format crate against the Go engine's own functions: every line of the vector files in `rust/testdata` is one
//! input and what Go made of it. `rust/testdata/README.md` says how each file is written.

use dostoevsky_format::checkpoint3::{append_checkpoint3, read_checkpoint3, Checkpoint3, CheckpointError};
use dostoevsky_format::frame3::*;
use dostoevsky_format::recordscan::RecordScanner;
use dostoevsky_format::{base64, crc32c, gotime};
use std::collections::BTreeMap;

fn unhex(h: &str) -> Vec<u8> {
    let h = h.trim_matches('-');
    (0..h.len() / 2)
        .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn x(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2 + 2);
    s.push('-');
    for byte in b {
        s.push_str(&format!("{byte:02x}"));
    }
    s.push('-');
    s
}

fn rows(file: &'static str) -> impl Iterator<Item = &'static str> {
    file.lines().filter(|l| !l.starts_with('#'))
}

fn verdict<T>(r: &Result<T, Frame3Error>) -> &'static str {
    match r {
        Ok(_) => "ok",
        Err(Frame3Error::Short) => "short",
        Err(Frame3Error::Header) => "header",
        Err(Frame3Error::Checksum) => "checksum",
        Err(Frame3Error::Invalid(_)) => "invalid",
        Err(Frame3Error::Limit) => "limit",
    }
}

/// The error's text in hex, or `--` for none: the Go messages are kept word for word.
fn message<T, E: std::fmt::Display>(r: &Result<T, E>) -> String {
    match r {
        Ok(_) => "--".into(),
        Err(e) => x(e.to_string().as_bytes()),
    }
}

#[test]
fn crc32c_matches_go() {
    let mut seen = 0;
    for line in rows(include_str!("../../../testdata/crc32c-go.txt")) {
        let f: Vec<&str> = line.split(' ').collect();
        let (b, cut) = (unhex(f[0]), f[2].parse::<usize>().unwrap());
        assert_eq!(crc32c::checksum(&b).to_string(), f[1], "{line}");
        assert_eq!(
            crc32c::update(crc32c::checksum(&b[..cut]), &b[cut..]).to_string(),
            f[3],
            "{line}"
        );
        seen += 1;
    }
    assert_eq!(seen, 300);
}

#[test]
fn frames_match_go() {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for line in rows(include_str!("../../../testdata/frame3-go.txt")) {
        let f: Vec<&str> = line.split(' ').collect();
        let n = |i: usize| f[i].parse::<u64>().unwrap();
        let mut key = f[0].to_string();
        match f[0] {
            "A" => {
                let mut out = b"xy".to_vec();
                append_frame3(&mut out, n(1) as u8, &unhex(f[2])).unwrap();
                assert_eq!(x(&out[2..]), f[3], "{line}");
            }
            "REC" => {
                let (k, id, v) = (unhex(f[4]), unhex(f[5]), unhex(f[6]));
                let r = Record3 {
                    partition: n(1) as u32,
                    offset: n(2),
                    time: f[3].parse().unwrap(),
                    key: &k,
                    id: &id,
                    value: &v,
                };
                let mut out = Vec::new();
                append_record3(&mut out, &r).unwrap();
                assert_eq!(x(&out), f[7], "{line}");
                let (typ, body, size) = read_frame3(&out).unwrap();
                assert_eq!(
                    (typ, size, parse_record3(body)),
                    (F3_RECORD, out.len(), Ok(r)),
                    "{line}"
                );
            }
            "COM" => {
                let (g, s) = (unhex(f[2]), unhex(f[6]));
                let c = Commit3 {
                    partition: n(1) as u32,
                    group: &g,
                    next: n(3),
                    expected: (f[4] == "true").then(|| n(5)),
                    state: &s,
                };
                let mut out = Vec::new();
                append_commit3(&mut out, &c).unwrap();
                assert_eq!(x(&out), f[7], "{line}");
                assert_eq!(parse_commit3(read_frame3(&out).unwrap().1), Ok(c), "{line}");
            }
            "TOP" => {
                let t = unhex(f[3]);
                let d = TopicDef3 {
                    partition: n(1) as u32,
                    number: n(2) as u32,
                    topic: &t,
                };
                let mut out = Vec::new();
                append_topic_def3(&mut out, &d).unwrap();
                assert_eq!(x(&out), f[4], "{line}");
                assert_eq!(parse_topic_def3(read_frame3(&out).unwrap().1), Ok(d), "{line}");
            }
            "LIMIT" => {
                let big = vec![0u8; u16::MAX as usize + 1];
                let mut out = b"kept".to_vec();
                let got = match f[1] {
                    "record-key" => append_record3(
                        &mut out,
                        &Record3 {
                            key: &big,
                            ..Default::default()
                        },
                    ),
                    "record-id" => append_record3(
                        &mut out,
                        &Record3 {
                            id: &big,
                            ..Default::default()
                        },
                    ),
                    "record-value" => append_record3(
                        &mut out,
                        &Record3 {
                            value: &vec![0; FRAME3_MAX - 24 + 1],
                            ..Default::default()
                        },
                    ),
                    "record-largest" => append_record3(
                        &mut out,
                        &Record3 {
                            value: &vec![0; FRAME3_MAX - 24],
                            ..Default::default()
                        },
                    ),
                    "commit-group" => append_commit3(
                        &mut out,
                        &Commit3 {
                            group: &big,
                            ..Default::default()
                        },
                    ),
                    "commit-state" => append_commit3(
                        &mut out,
                        &Commit3 {
                            state: &vec![0; FRAME3_MAX - 22],
                            ..Default::default()
                        },
                    ),
                    "topic" => append_topic_def3(
                        &mut out,
                        &TopicDef3 {
                            topic: &big,
                            ..Default::default()
                        },
                    ),
                    "frame" => append_frame3(&mut out, 1, &vec![0; FRAME3_MAX + 1]),
                    "frame-largest" => append_frame3(&mut out, 1, &vec![0; FRAME3_MAX]),
                    other => panic!("unknown limit {other}"),
                };
                assert_eq!(verdict(&got), f[2], "{line}");
                if got.is_err() {
                    assert_eq!(out, b"kept", "{line}: a refused append must leave the buffer as it was");
                }
            }
            "R" => {
                let b = unhex(f[1]);
                let got = read_frame3(&b);
                let (typ, size) = got.as_ref().map_or((0, 0), |g| (g.0, g.2));
                assert_eq!(
                    format!("R {} {} {typ} {size} {}", f[1], verdict(&got), message(&got)),
                    line
                );
                key = format!("R {}", f[2]);
            }
            "PR" => {
                let b = unhex(f[1]);
                let got = parse_record3(&b);
                let r = got.clone().unwrap_or_default();
                let want = format!(
                    "PR {} {} {} {} {} {} {} {} {}",
                    f[1],
                    verdict(&got),
                    r.partition,
                    r.offset,
                    r.time,
                    x(r.key),
                    x(r.id),
                    x(r.value),
                    message(&got)
                );
                assert_eq!(want, line);
                key = format!("PR {}", f[2]);
            }
            "PC" => {
                let b = unhex(f[1]);
                let got = parse_commit3(&b);
                let c = got.clone().unwrap_or_default();
                let want = format!(
                    "PC {} {} {} {} {} {} {} {} {}",
                    f[1],
                    verdict(&got),
                    c.partition,
                    x(c.group),
                    c.next,
                    c.expected.is_some(),
                    c.expected.unwrap_or(0),
                    x(c.state),
                    message(&got)
                );
                assert_eq!(want, line);
                key = format!("PC {}", f[2]);
            }
            "PT" => {
                let b = unhex(f[1]);
                let got = parse_topic_def3(&b);
                let d = got.clone().unwrap_or_default();
                assert_eq!(
                    format!(
                        "PT {} {} {} {} {} {}",
                        f[1],
                        verdict(&got),
                        d.partition,
                        d.number,
                        x(d.topic),
                        message(&got)
                    ),
                    line
                );
                key = format!("PT {}", f[2]);
            }
            other => panic!("unknown row {other}"),
        }
        *counts.entry(key).or_default() += 1;
    }
    for (kind, least) in [
        ("R ok", 200),
        ("R short", 200),
        ("R header", 100),
        ("R checksum", 100),
        ("R invalid", 10),
        ("PR invalid", 50),
        ("PC invalid", 50),
        ("PT invalid", 5),
        ("LIMIT", 9),
    ] {
        assert!(counts.get(kind).copied().unwrap_or(0) >= least, "{kind}: {counts:?}");
    }
}

fn dump(cp: &Checkpoint3) -> String {
    let mut s = format!("{} {} {} T", cp.id, cp.segment, x(&cp.config));
    for t in &cp.topics {
        s.push_str(&format!(" {},{},{}", t.partition, t.number, x(&t.topic)));
    }
    s.push_str(" P");
    for p in &cp.partitions {
        s.push_str(&format!(" {},{},{}", p.partition, p.base, p.next));
    }
    s.push_str(" G");
    for g in &cp.groups {
        let state = g.state.as_deref().map_or("nil".to_string(), x);
        s.push_str(&format!(" {},{},{},{state}", g.partition, x(&g.group), g.next));
    }
    s
}

#[test]
fn checkpoints_match_go() {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for line in rows(include_str!("../../../testdata/checkpoint3-go.txt")) {
        let f: Vec<&str> = line.splitn(2, ' ').collect();
        let b = unhex(f[0]);
        let got = read_checkpoint3(&b);
        let (kind, size, cp) = match &got {
            Ok((cp, size)) => ("ok", *size, cp.clone()),
            Err(CheckpointError::Torn(_)) => ("torn", 0, Checkpoint3::default()),
            Err(CheckpointError::Invalid(_)) => ("invalid", 0, Checkpoint3::default()),
            Err(CheckpointError::Limit) => panic!("a reader cannot hit a limit"),
        };
        assert_eq!(format!("{} {kind} {size} {} {}", f[0], message(&got), dump(&cp)), line);
        if got.is_ok() {
            // What was read writes back as the bytes it was read from.
            let mut again = b"z".to_vec();
            append_checkpoint3(&mut again, &cp).unwrap();
            assert_eq!(&again[1..], &b[..size], "{line}");
        }
        *counts.entry(kind).or_default() += 1;
    }
    assert!(
        counts["ok"] >= 40 && counts["torn"] >= 100 && counts["invalid"] >= 150,
        "{counts:?}"
    );
}

#[test]
fn a_refused_checkpoint_leaves_the_buffer_as_it_was() {
    let cp = Checkpoint3 {
        groups: vec![
            Default::default(),
            dostoevsky_format::checkpoint3::CpGroup3 {
                group: vec![b'g'; 70_000],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut out = b"kept".to_vec();
    assert_eq!(append_checkpoint3(&mut out, &cp), Err(CheckpointError::Limit));
    assert_eq!(out, b"kept");
}

#[test]
fn times_match_go() {
    let (mut accepted, mut refused) = (0, 0);
    for line in rows(include_str!("../../../testdata/gotime-go.txt")) {
        let f: Vec<&str> = line.split(' ').collect();
        let text = unhex(f[0]);
        let got = match gotime::parse_rfc3339(&text) {
            // Go reads the JSON word null as "no time"; that is the caller's case, not a time.
            _ if text == b"null" => continue,
            Some(t) => {
                accepted += 1;
                format!("{} ok {} {}", f[0], t.seconds, t.nanoseconds)
            }
            None => {
                refused += 1;
                format!("{} err", f[0])
            }
        };
        assert_eq!(got, line, "{}", String::from_utf8_lossy(&text));
    }
    assert!(
        accepted > 300 && refused > 3000,
        "{accepted} accepted, {refused} refused"
    );
}

#[test]
fn base64_matches_go() {
    let (mut accepted, mut refused) = (0, 0);
    // What a strict decoder would get wrong: text whose last group leaves bits that are not zero, and text with
    // line ends in it. Go takes both, so the rows must hold both and this decoder must take them.
    let (mut loose_bits, mut line_ends) = (0, 0);
    for line in rows(include_str!("../../../testdata/base64-go.txt")) {
        let f: Vec<&str> = line.split(' ').collect();
        let input = String::from_utf8(unhex(f[0])).unwrap();
        let got = match base64::decode(&input) {
            Some(out) => {
                accepted += 1;
                let plain: String = input.chars().filter(|c| *c != '\r' && *c != '\n').collect();
                loose_bits += usize::from(base64::encode(&out) != plain);
                line_ends += usize::from(plain != input);
                format!("{} ok {}", f[0], x(&out))
            }
            None => {
                refused += 1;
                format!("{} err", f[0])
            }
        };
        assert_eq!(got, line, "{input:?}");
    }
    assert!(
        accepted > 300 && refused > 500 && loose_bits >= 8 && line_ends >= 100,
        "{accepted} accepted, {refused} refused, {loose_bits} with loose bits, {line_ends} with line ends"
    );
    // The same by name, so that the rule is readable here and fails here if the rows are ever lost.
    assert_eq!(
        base64::decode("Zh==").as_deref(),
        Some(&b"f"[..]),
        "the unused bits are ignored"
    );
    assert_eq!(base64::decode("Zm9=").as_deref(), Some(&b"fo"[..]));
    assert_eq!(
        base64::decode("Z\ng=\r\n=").as_deref(),
        Some(&b"f"[..]),
        "line ends are skipped anywhere"
    );
    assert_eq!(base64::decode("\r\n").as_deref(), Some(&b""[..]));
    assert_eq!(base64::decode("Zg==Zg=="), None, "nothing may follow the padding");
    assert_eq!(base64::decode("Zm9v Ymfy"), None, "a space is not a line end");
}

#[test]
fn the_frame_time_field_matches_go() {
    let (mut held, mut refused, mut back) = (0, 0, 0);
    for line in rows(include_str!("../../../testdata/frametime-go.txt")) {
        let f: Vec<&str> = line.split(' ').collect();
        let got = match f[0] {
            "T" => {
                let at = gotime::Instant {
                    seconds: f[1].parse().unwrap(),
                    nanoseconds: f[2].parse().unwrap(),
                };
                match time_to_nanos(at) {
                    Ok(n) => {
                        held += 1;
                        if n != ZERO_TIME {
                            assert_eq!(nanos_to_time(n), at, "{line}");
                        }
                        format!("T {} {} ok {n}", f[1], f[2])
                    }
                    Err(TimeOutOfRange(refused_time)) => {
                        refused += 1;
                        assert_eq!(refused_time, at);
                        format!("T {} {} err", f[1], f[2])
                    }
                }
            }
            "N" => {
                back += 1;
                let at = nanos_to_time(f[1].parse().unwrap());
                format!("N {} {} {}", f[1], at.seconds, at.nanoseconds)
            }
            other => panic!("unknown row {other}"),
        };
        assert_eq!(got, line);
    }
    assert!(
        held > 300 && refused > 150 && back > 300,
        "{held} held, {refused} refused, {back} converted back"
    );
    assert_eq!(time_to_nanos(ZERO_INSTANT), Ok(ZERO_TIME));
    assert_eq!(nanos_to_time(ZERO_TIME), ZERO_INSTANT);
}

#[test]
fn the_record_scanner_matches_go() {
    let mut scanner = RecordScanner::default();
    let (mut accepted, mut refused) = (0, 0);
    for line in rows(include_str!("../../../testdata/recordscan-go.txt")) {
        let f: Vec<&str> = line.split(' ').collect();
        let b = unhex(f[0]);
        let got = match scanner.scan(&b) {
            Some(s) => {
                accepted += 1;
                format!(
                    "{} ok {} {} {} {}",
                    f[0],
                    x(s.topic.as_bytes()),
                    s.number,
                    s.offset,
                    x(s.id.as_bytes())
                )
            }
            None => {
                refused += 1;
                format!("{} no", f[0])
            }
        };
        assert_eq!(got, line, "{}", String::from_utf8_lossy(&b));
    }
    assert!(
        accepted > 60 && refused > 1000,
        "{accepted} accepted, {refused} refused"
    );
}

fn list(field: &str) -> Vec<&str> {
    let inner = field.trim_start_matches('[').trim_end_matches(']');
    if inner.is_empty() {
        Vec::new()
    } else {
        inner.split(',').collect()
    }
}

#[test]
fn the_id_table_and_filter_match_go() {
    use dostoevsky_format::idfilter::{filter_positions, make_id_filter, mix_filter_hash};
    use dostoevsky_format::segindex::{id_hash, IdTable};
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for line in rows(include_str!("../../../testdata/idtable-go.txt")) {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "H" => assert_eq!(id_hash(f[1].parse().unwrap(), &unhex(f[2])).to_string(), f[3], "{line}"),
            "P" => {
                let (h, bits): (u64, u32) = (f[1].parse().unwrap(), f[2].parse().unwrap());
                let (pos, step) = filter_positions(h, bits);
                assert_eq!(format!("P {h} {bits} {} {pos} {step}", mix_filter_hash(h)), line);
            }
            "T" => {
                let mut table = IdTable::default();
                for add in list(f[3]) {
                    let (h, off) = add.split_once(':').unwrap();
                    table.add(h.parse().unwrap(), off.parse().unwrap());
                }
                assert_eq!(
                    (table.len().to_string().as_str(), table.keys.len().to_string().as_str()),
                    (f[1], f[2]),
                    "{line}"
                );
                // The order of each() and of every lookup is the order Go visits in.
                let mut each = Vec::new();
                table.each(|h, off| each.push(format!("{h}:{off}")));
                assert_eq!(format!("[{}]", each.join(",")), f[4], "{line}");
                for look in list(f[5]) {
                    let (query, want) = look.split_once('=').unwrap();
                    let (h, stop) = query.split_once('/').unwrap();
                    let stop: usize = stop.parse().unwrap();
                    let mut found = Vec::new();
                    table.lookup(h.parse().unwrap(), |off| {
                        found.push(off.to_string());
                        found.len() == stop
                    });
                    assert_eq!(found.join("+"), want, "{look} in {line}");
                }
                let filter = make_id_filter(&table);
                assert_eq!(x(&filter.0), f[6], "{line}");
                for check in list(f[7]) {
                    let (h, want) = check.split_once('=').unwrap();
                    assert_eq!(
                        filter.contains(mix_filter_hash(h.parse().unwrap())).to_string(),
                        want,
                        "{check} in {line}"
                    );
                }
            }
            other => panic!("unknown row {other}"),
        }
        *counts.entry(f[0]).or_default() += 1;
    }
    assert_eq!((counts["H"], counts["T"], counts["P"]), (200, 60, 200));
}

#[test]
fn the_segment_index_matches_go() {
    use dostoevsky_format::segindex::{decode_seg_index, encode_seg_index, IdxFooterInfo, SegIndex};
    let footer = |text: &str| {
        let n: Vec<u64> = text.split(',').map(|v| v.parse().unwrap()).collect();
        IdxFooterInfo {
            segment: n[0],
            size: n[1],
            last_pos: n[2],
            last_crc: n[3] as u32,
        }
    };
    let (mut built, mut accepted, mut refused) = (0, 0, 0);
    for line in rows(include_str!("../../../testdata/segindex-go.txt")) {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "X" => {
                let start = list(f[1])
                    .iter()
                    .map(|e| e.split_once('=').unwrap())
                    .map(|(p, n)| (p.parse().unwrap(), n.parse().unwrap()))
                    .collect();
                let mut index = SegIndex::new(start);
                for rec in list(f[2]) {
                    let r: Vec<&str> = rec.split(':').collect();
                    let id = unhex(r[3]);
                    let record = Record3 {
                        partition: r[0].parse().unwrap(),
                        offset: r[1].parse().unwrap(),
                        time: r[2].parse().unwrap(),
                        id: &id,
                        ..Default::default()
                    };
                    index.observe(&record, r[4].parse().unwrap());
                }
                assert_eq!(
                    x(&encode_seg_index(&index, footer(f[3]))),
                    f[4],
                    "{}",
                    &line[..line.len().min(200)]
                );
                for query in list(f[5]) {
                    let (q, want) = query.split_once('=').unwrap();
                    let got = match q.split_once('@') {
                        None => {
                            let pid = q.parse().unwrap();
                            format!("{}/{}", index.first(pid), index.count(pid))
                        }
                        Some((pid, off)) => {
                            let at = index.find(pid.parse().unwrap(), off.parse().unwrap());
                            format!("{}/{}", at.unwrap_or(0), at.is_some())
                        }
                    };
                    assert_eq!(got, want, "{query}");
                }
                built += 1;
            }
            "D" => {
                let b = unhex(f[1]);
                let got = decode_seg_index(&b);
                let (foot, text, again) = match &got {
                    Ok((index, foot)) => {
                        accepted += 1;
                        (*foot, "--".to_string(), x(&encode_seg_index(index, *foot)))
                    }
                    Err((foot, e)) => {
                        refused += 1;
                        (*foot, x(e.to_string().as_bytes()), "--".to_string())
                    }
                };
                let want = format!(
                    "D {} {},{},{},{} {text} {again}",
                    f[1], foot.segment, foot.size, foot.last_pos, foot.last_crc
                );
                assert_eq!(want, line, "{}", &line[..line.len().min(200)]);
                if let Ok((index, foot)) = &got {
                    assert_eq!(
                        encode_seg_index(index, *foot),
                        b,
                        "what was read writes back as the bytes it was read from"
                    );
                    assert!(index.same_as(index, *foot));
                }
            }
            other => panic!("unknown row {other}"),
        }
    }
    assert!(
        built == 22 && accepted >= 22 && refused > 300,
        "{built} built, {accepted} accepted, {refused} refused"
    );
}
