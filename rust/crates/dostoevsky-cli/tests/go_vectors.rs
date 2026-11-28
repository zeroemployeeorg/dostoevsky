//! Checks the driver's request binding against the Go driver's own, row for row. The rows are written by
//! `rust/testdata/driver_vectors_test.go.txt` inside the Go driver's package.

use dostoevsky_cli::conform::{decode, Request, Slice};
use dostoevsky_format::gojson::{self, ParseError, Value};

fn text(s: &str) -> Value {
    Value::text(s)
}

fn bytes(b: &Slice<u8>) -> Value {
    Value::bytes(b.get())
}

fn list<T>(s: &Slice<T>, element: impl Fn(&T) -> Value) -> Value {
    match s.get() {
        None => Value::Null,
        Some(items) => Value::Array(items.iter().map(element).collect()),
    }
}

fn map<T>(m: &Option<Vec<(String, T)>>, element: impl Fn(&T) -> Value) -> Value {
    match m {
        None => Value::Null,
        Some(entries) => Value::map(entries.iter().map(|(k, v)| (k.clone(), element(v))).collect()),
    }
}

/// The request as Go's `json.Marshal` writes the driver's request struct.
fn marshal(q: &Request) -> Vec<u8> {
    let members: Vec<(&str, Value)> = vec![
        ("op", text(&q.op)),
        ("dir", text(&q.dir)),
        ("segmented", Value::Bool(q.segmented)),
        ("node", text(&q.node)),
        ("nodes", list(&q.nodes, |s| text(s))),
        ("topics", map(&q.topics, |n| Value::int(*n))),
        (
            "items",
            list(&q.items, |m| {
                Value::Object(vec![
                    ("topic".into(), text(&m.topic)),
                    ("key".into(), text(&m.key)),
                    ("value".into(), bytes(&m.value)),
                    ("id".into(), text(&m.id)),
                ])
            }),
        ),
        ("group", text(&q.group)),
        ("offset", Value::int(q.offset)),
        ("next", Value::int(q.next)),
        ("state", bytes(&q.state)),
        ("records", Value::int(q.records)),
        ("bytes", Value::int(q.bytes)),
        (
            "floors",
            list(&q.floors, |f| {
                Value::Object(vec![
                    ("topic".into(), text(&f.topic)),
                    ("partition".into(), Value::int(f.partition)),
                    ("floor".into(), Value::int(f.floor)),
                ])
            }),
        ),
        ("dest", text(&q.dest)),
        ("discard_later", Value::Bool(q.discard_later)),
        ("topic", text(&q.topic)),
        ("partition", Value::int(q.partition)),
        ("key", text(&q.key)),
        ("value", bytes(&q.value)),
        ("id", text(&q.id)),
        ("name", text(&q.name)),
        ("version", Value::int(q.version)),
        (
            "steps",
            list(&q.steps, |w| {
                Value::Object(vec![
                    ("kind".into(), text(&w.kind)),
                    ("name".into(), text(&w.name)),
                    ("attempts".into(), Value::int(w.attempts)),
                    ("backoff_ms".into(), Value::int(w.backoff_ms)),
                ])
            }),
        ),
        ("run", text(&q.run)),
        ("queue", text(&q.queue)),
        ("input", bytes(&q.input)),
        ("attrs", map(&q.attrs, |s| text(s))),
        ("lease_ms", Value::int(q.lease_ms)),
        ("token", text(&q.token)),
        ("failure", bytes(&q.failure)),
        ("step", Value::int(q.step)),
        ("after", Value::int(q.after)),
        ("limit", Value::int(q.limit)),
        ("start", Value::int(q.start)),
        ("seed", Value::Number(q.seed.to_string())),
        ("nanoseconds", Value::int(q.nanoseconds)),
        ("point", text(&q.point)),
        ("kill", Value::Bool(q.kill)),
        ("skip", Value::int(q.skip)),
    ];
    let mut line = gojson::encode_line(&Value::Object(
        members.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
    ));
    line.pop(); // the newline
    line
}

fn unhex(field: &str) -> Vec<u8> {
    let digits = field.trim_matches('-').as_bytes();
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn requests_bind_as_the_go_driver_binds_them() {
    let rows = include_str!("../../../testdata/driver-requests-go.txt");
    let (mut bound, mut refused, mut empty) = (0, 0, 0);
    // Rows that show each thing Go's binding accepts beyond the plain case: these must be among the vectors.
    let (mut byte_arrays, mut folded, mut nulls, mut repeated) = (0, 0, 0, 0);
    for row in rows.lines().filter(|l| !l.starts_with('#')) {
        let (input, want) = row.split_once(' ').unwrap();
        let input = unhex(input);
        let got = match gojson::parse_prefix(&input, true) {
            Err(ParseError::Incomplete) if input.iter().all(u8::is_ascii_whitespace) => {
                empty += 1;
                "eof".to_string()
            }
            Err(_) => "err".to_string(),
            Ok((value, _)) => match decode(&value) {
                Err(_) => "err".to_string(),
                Ok(q) => {
                    bound += 1;
                    let shown = String::from_utf8_lossy(&input);
                    byte_arrays += usize::from(shown.contains("\"value\":[") && !shown.contains("items"));
                    folded += usize::from(shown.contains('\u{17f}') || shown.contains('\u{212a}'));
                    nulls += usize::from(shown.contains("null"));
                    repeated += usize::from(shown.matches("\"nodes\":").count() > 1);
                    format!("ok {}", String::from_utf8(marshal(&q)).unwrap())
                }
            },
        };
        refused += usize::from(got == "err");
        assert_eq!(got, want, "{}", String::from_utf8_lossy(&input));
    }
    assert_eq!((bound, refused, empty), (5683, 1935, 3));
    assert!(
        byte_arrays > 500 && folded > 30 && nulls > 2000 && repeated > 500,
        "{byte_arrays} byte arrays, {folded} folded names, {nulls} with null, {repeated} with a repeated key"
    );
}
