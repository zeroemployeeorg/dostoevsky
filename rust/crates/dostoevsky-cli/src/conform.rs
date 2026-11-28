//! The conformance driver for this engine. Mirrors `cmd/dost-conform/main.go`: one JSON request in, one JSON reply
//! line out, until end of input. The protocol is described in `cmd/dost-conform/README.md`.
//!
//! Besides the engine requests it serves the test seam (`dostoevsky_core::seam`): `fix_inputs` (`start`, `step`,
//! and `seed` as an unsigned 64-bit number), `advance` (`nanoseconds`), `arm` (`point`, `kill`, `skip`) and `disarm`
//! (which replies `reached`: whether the armed point fired). `fix_inputs` must come before anything is opened. `arm`
//! refuses a name that is not one of the engine's points, and refuses to arm again before `disarm` has reported the
//! point armed before.
//!
//! A request whose module is not ported yet answers with the error `other` and names the row that brings it. The
//! parts that do not depend on the engine are complete and tested: binding a request as Go's decoder binds it, the
//! order of the checks, the error codes, and the reply bytes.

use dostoevsky_core::errors::Error;
use dostoevsky_core::seam;
use dostoevsky_format::base64;
use dostoevsky_format::gojson::{self, ParseError, Value};
use std::io::{BufRead, Write};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Message {
    pub topic: String,
    pub key: String,
    pub value: Slice<u8>,
    pub id: String,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Floor {
    pub topic: String,
    pub partition: i64,
    pub floor: i64,
}

/// One request. A field the request does not carry keeps its zero value, as in Go.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Request {
    pub op: String,
    pub dir: String,
    pub segmented: bool,
    pub node: String,
    pub nodes: Slice<String>,
    pub topics: Option<Vec<(String, i64)>>,
    pub items: Slice<Message>,
    pub group: String,
    pub offset: i64,
    pub next: i64,
    pub state: Slice<u8>,
    pub records: i64,
    pub bytes: i64,
    pub floors: Slice<Floor>,
    pub dest: String,
    pub discard_later: bool,
    pub topic: String,
    pub partition: i64,
    pub key: String,
    pub value: Slice<u8>,
    pub id: String,
    // The workflow requests (`wf_...`). `step` is also the clock step of `fix_inputs`.
    pub name: String,
    pub version: i64,
    pub steps: Slice<WorkflowStep>,
    pub run: String,
    pub queue: String,
    pub input: Slice<u8>,
    pub attrs: Option<Vec<(String, String)>>,
    pub lease_ms: i64,
    pub token: String,
    pub failure: Slice<u8>,
    pub step: i64,
    pub after: i64,
    pub limit: i64,
    // The seam requests.
    pub start: i64,
    pub seed: u64,
    pub nanoseconds: i64,
    pub point: String,
    pub kill: bool,
    pub skip: i64,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct WorkflowStep {
    pub kind: String,
    pub name: String,
    pub attempts: i64,
    pub backoff_ms: i64,
}

type Decoded<T> = Result<T, String>;

/// A Go slice as `encoding/json` leaves it. Go decodes an array into the slice the field already holds: element
/// by element over the old ones, without clearing them first, and it cuts the slice to the new length without
/// dropping what lay beyond. So a key given twice does not simply take its last value: a `null` element, or a
/// member an object leaves out, keeps what an earlier value put there. `spare` holds the elements beyond the
/// length, which a longer array brings back.
#[derive(Debug, Default, Clone)]
pub struct Slice<T> {
    live: Option<Vec<T>>,
    spare: Vec<T>,
}

impl<T> Slice<T> {
    /// The slice: `None` for Go's nil.
    pub fn get(&self) -> Option<&[T]> {
        self.live.as_deref()
    }

    /// Go's `null`: the field becomes nil and the old elements are gone.
    fn clear(&mut self) {
        self.live = None;
        self.spare.clear();
    }

    /// A value that replaces the slice outright, as a base64 string replaces a byte slice.
    fn set(&mut self, items: Vec<T>) {
        self.live = Some(items);
        self.spare.clear();
    }
}

impl<T: Default> Slice<T> {
    /// Go's `decodeState.array` for a slice.
    fn array(&mut self, values: &[Value], element: impl Fn(&mut T, &Value) -> Decoded<()>) -> Decoded<()> {
        if values.is_empty() {
            // Go replaces the slice with a new empty one.
            self.set(Vec::new());
            return Ok(());
        }
        let mut all = self.live.take().unwrap_or_default();
        all.append(&mut self.spare);
        for (i, v) in values.iter().enumerate() {
            if i == all.len() {
                all.push(T::default());
            }
            element(&mut all[i], v)?;
        }
        self.spare = all.split_off(values.len());
        self.live = Some(all);
        Ok(())
    }
}

impl<T: PartialEq> PartialEq for Slice<T> {
    fn eq(&self, other: &Self) -> bool {
        self.live == other.live
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn wrong<T>(field: &str, v: &Value, want: &str) -> Decoded<T> {
    Err(format!(
        "json: cannot unmarshal {} into Go struct field {field} of type {want}",
        kind(v)
    ))
}

// Each setter leaves the field alone for null, as Go does.

fn set_string(to: &mut String, field: &str, v: &Value) -> Decoded<()> {
    match v {
        Value::Null => Ok(()),
        Value::String(s) => {
            *to = s.clone();
            Ok(())
        }
        _ => wrong(field, v, "string"),
    }
}

fn set_bool(to: &mut bool, field: &str, v: &Value) -> Decoded<()> {
    match v {
        Value::Null => Ok(()),
        Value::Bool(b) => {
            *to = *b;
            Ok(())
        }
        _ => wrong(field, v, "bool"),
    }
}

fn set_int(to: &mut i64, field: &str, v: &Value) -> Decoded<()> {
    match v {
        Value::Null => Ok(()),
        // Go accepts only a literal that is an integer as written: no fraction and no exponent.
        Value::Number(n) => match n.parse::<i64>() {
            Ok(i) => {
                *to = i;
                Ok(())
            }
            Err(_) => Err(format!(
                "json: cannot unmarshal number {n} into Go struct field {field} of type int"
            )),
        },
        _ => wrong(field, v, "int"),
    }
}

/// An unsigned field of the given width. Go refuses a sign, so `-0` is refused here although it is zero.
fn set_unsigned(to: &mut u64, limit: u64, field: &str, v: &Value) -> Decoded<()> {
    match v {
        Value::Null => Ok(()),
        Value::Number(n) => match n.parse::<u64>() {
            Ok(i) if i <= limit => {
                *to = i;
                Ok(())
            }
            _ => Err(format!(
                "json: cannot unmarshal number {n} into Go struct field {field} of type uint"
            )),
        },
        _ => wrong(field, v, "uint"),
    }
}

/// A byte slice: base64 text, or an array of numbers from 0 to 255, which Go reads as it reads any slice.
fn set_bytes(to: &mut Slice<u8>, field: &str, v: &Value) -> Decoded<()> {
    match v {
        Value::Null => {
            to.clear();
            Ok(())
        }
        Value::String(s) => match base64::decode(s) {
            Some(b) => {
                to.set(b);
                Ok(())
            }
            None => Err(format!("illegal base64 data in field {field}")),
        },
        Value::Array(values) => to.array(values, |byte, v| {
            let mut n = u64::from(*byte);
            set_unsigned(&mut n, 255, field, v)?;
            *byte = n as u8;
            Ok(())
        }),
        _ => wrong(field, v, "[]uint8"),
    }
}

fn set_list<T: Default>(
    to: &mut Slice<T>,
    field: &str,
    v: &Value,
    element: impl Fn(&mut T, &Value) -> Decoded<()>,
) -> Decoded<()> {
    match v {
        Value::Null => {
            to.clear();
            Ok(())
        }
        Value::Array(values) => to.array(values, element),
        _ => wrong(field, v, "slice"),
    }
}

/// A Go map. `null` makes it nil. An object adds to the map already there; each value starts from zero, so a key
/// given twice keeps one entry with the last value, and a `null` value leaves zero.
fn set_map<T: Default>(
    to: &mut Option<Vec<(String, T)>>,
    field: &str,
    v: &Value,
    element: impl Fn(&mut T, &Value) -> Decoded<()>,
) -> Decoded<()> {
    match v {
        Value::Null => {
            *to = None;
            Ok(())
        }
        Value::Object(members) => {
            let map = to.get_or_insert_with(Vec::new);
            for (key, v) in members {
                let mut value = T::default();
                element(&mut value, v)?;
                match map.iter_mut().find(|entry| entry.0 == *key) {
                    Some(entry) => entry.1 = value,
                    None => map.push((key.clone(), value)),
                }
            }
            Ok(())
        }
        _ => wrong(field, v, "map"),
    }
}

/// Go's `foldName`: ASCII letters to upper case, and every other character to the smallest character of its simple
/// case-folding orbit. Two characters outside ASCII share an orbit with an ASCII letter: U+017F (long s) with `S`
/// and U+212A (the kelvin sign) with `K`. No other character can be part of a match with an ASCII field name, so the
/// rest pass through unchanged.
fn fold(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '\u{17f}' => 'S',
            '\u{212a}' => 'K',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}

/// Go matches a key to a field by its exact name first and otherwise by its folded name.
fn field_of<'a>(key: &str, names: &[&'a str]) -> Option<&'a str> {
    names.iter().find(|n| **n == key).copied().or_else(|| {
        let folded = fold(key);
        names.iter().find(|n| n.to_ascii_uppercase() == folded).copied()
    })
}

/// The members of an object that is bound to a struct. `None` for `null`, which leaves the struct as it is.
fn members<'a>(v: &'a Value, what: &str) -> Decoded<Option<&'a [(String, Value)]>> {
    match v {
        Value::Null => Ok(None),
        Value::Object(m) => Ok(Some(m)),
        _ => wrong(what, v, "struct"),
    }
}

fn unknown<T>(key: &str) -> Decoded<T> {
    Err(format!("json: unknown field {key:?}"))
}

fn message(m: &mut Message, v: &Value) -> Decoded<()> {
    for (key, v) in members(v, "items")?.unwrap_or_default() {
        match field_of(key, &["topic", "key", "value", "id"]) {
            Some("topic") => set_string(&mut m.topic, "topic", v)?,
            Some("key") => set_string(&mut m.key, "key", v)?,
            Some("value") => set_bytes(&mut m.value, "value", v)?,
            Some("id") => set_string(&mut m.id, "id", v)?,
            _ => return unknown(key),
        }
    }
    Ok(())
}

fn floor(f: &mut Floor, v: &Value) -> Decoded<()> {
    for (key, v) in members(v, "floors")?.unwrap_or_default() {
        match field_of(key, &["topic", "partition", "floor"]) {
            Some("topic") => set_string(&mut f.topic, "topic", v)?,
            Some("partition") => set_int(&mut f.partition, "partition", v)?,
            Some("floor") => set_int(&mut f.floor, "floor", v)?,
            _ => return unknown(key),
        }
    }
    Ok(())
}

fn workflow_step(w: &mut WorkflowStep, v: &Value) -> Decoded<()> {
    for (key, v) in members(v, "steps")?.unwrap_or_default() {
        match field_of(key, &["kind", "name", "attempts", "backoff_ms"]) {
            Some("kind") => set_string(&mut w.kind, "kind", v)?,
            Some("name") => set_string(&mut w.name, "name", v)?,
            Some("attempts") => set_int(&mut w.attempts, "attempts", v)?,
            Some("backoff_ms") => set_int(&mut w.backoff_ms, "backoff_ms", v)?,
            _ => return unknown(key),
        }
    }
    Ok(())
}

const FIELDS: [&str; 40] = [
    "op",
    "dir",
    "segmented",
    "node",
    "nodes",
    "topics",
    "items",
    "group",
    "offset",
    "next",
    "state",
    "records",
    "bytes",
    "floors",
    "dest",
    "discard_later",
    "topic",
    "partition",
    "key",
    "value",
    "id",
    "name",
    "version",
    "steps",
    "run",
    "queue",
    "input",
    "attrs",
    "lease_ms",
    "token",
    "failure",
    "step",
    "after",
    "limit",
    "start",
    "seed",
    "nanoseconds",
    "point",
    "kill",
    "skip",
];

/// Binds one JSON value to a request as Go's decoder binds it to the driver's request struct with unknown fields
/// disallowed. `null` gives the zero request. A key is matched by its exact name and otherwise by its folded name. A
/// key given twice is bound twice, onto what the first left: see [`Slice`].
pub fn decode(v: &Value) -> Decoded<Request> {
    let mut q = Request::default();
    for (key, v) in members(v, "request")?.unwrap_or_default() {
        let Some(name) = field_of(key, &FIELDS) else {
            return unknown(key);
        };
        match name {
            "op" => set_string(&mut q.op, name, v)?,
            "dir" => set_string(&mut q.dir, name, v)?,
            "segmented" => set_bool(&mut q.segmented, name, v)?,
            "node" => set_string(&mut q.node, name, v)?,
            "nodes" => set_list(&mut q.nodes, name, v, |s, v| set_string(s, "nodes", v))?,
            "topics" => set_map(&mut q.topics, name, v, |n, v| set_int(n, "topics", v))?,
            "items" => set_list(&mut q.items, name, v, message)?,
            "group" => set_string(&mut q.group, name, v)?,
            "offset" => set_int(&mut q.offset, name, v)?,
            "next" => set_int(&mut q.next, name, v)?,
            "state" => set_bytes(&mut q.state, name, v)?,
            "records" => set_int(&mut q.records, name, v)?,
            "bytes" => set_int(&mut q.bytes, name, v)?,
            "floors" => set_list(&mut q.floors, name, v, floor)?,
            "dest" => set_string(&mut q.dest, name, v)?,
            "discard_later" => set_bool(&mut q.discard_later, name, v)?,
            "topic" => set_string(&mut q.topic, name, v)?,
            "partition" => set_int(&mut q.partition, name, v)?,
            "key" => set_string(&mut q.key, name, v)?,
            "value" => set_bytes(&mut q.value, name, v)?,
            "id" => set_string(&mut q.id, name, v)?,
            "name" => set_string(&mut q.name, name, v)?,
            "version" => set_int(&mut q.version, name, v)?,
            "steps" => set_list(&mut q.steps, name, v, workflow_step)?,
            "run" => set_string(&mut q.run, name, v)?,
            "queue" => set_string(&mut q.queue, name, v)?,
            "input" => set_bytes(&mut q.input, name, v)?,
            "attrs" => set_map(&mut q.attrs, name, v, |s, v| set_string(s, "attrs", v))?,
            "lease_ms" => set_int(&mut q.lease_ms, name, v)?,
            "token" => set_string(&mut q.token, name, v)?,
            "failure" => set_bytes(&mut q.failure, name, v)?,
            "step" => set_int(&mut q.step, name, v)?,
            "after" => set_int(&mut q.after, name, v)?,
            "limit" => set_int(&mut q.limit, name, v)?,
            "start" => set_int(&mut q.start, name, v)?,
            "seed" => set_unsigned(&mut q.seed, u64::MAX, name, v)?,
            "nanoseconds" => set_int(&mut q.nanoseconds, name, v)?,
            "point" => set_string(&mut q.point, name, v)?,
            "kill" => set_bool(&mut q.kill, name, v)?,
            "skip" => set_int(&mut q.skip, name, v)?,
            _ => unreachable!("every name in FIELDS is handled"),
        }
    }
    Ok(q)
}

/// The reply line for a result and an error. A result can come with an error: a refused repair still reports what
/// it found. Field order and omissions are those of the Go driver's reply struct.
pub fn reply(result: Option<Value>, error: Option<&Error>) -> Vec<u8> {
    let mut members = vec![("ok".to_string(), Value::Bool(error.is_none()))];
    if let Some(e) = error {
        members.push(("error".into(), Value::text(e.code())));
        if !e.message().is_empty() {
            members.push(("message".into(), Value::text(e.message())));
        }
    }
    if let Some(r) = result {
        members.push(("result".into(), r));
    }
    gojson::encode_line(&Value::Object(members))
}

/// The driver's state: at most one open node or one open workflow store. The node arrives with row 6 and the store
/// with row 10.
#[derive(Default)]
pub struct Driver {
    node: Option<std::convert::Infallible>,
    store: Option<std::convert::Infallible>,
}

impl Driver {
    /// Performs one request. The order of the checks is the Go driver's.
    pub fn run(&mut self, q: &Request) -> (Option<Value>, Result<(), Error>) {
        match q.op.as_str() {
            "fix_inputs" => {
                if self.node.is_some() || self.store.is_some() {
                    return (
                        None,
                        Err(Error::other("dost-conform: fix_inputs comes before anything is opened")),
                    );
                }
                seam::fix(q.start, q.step, q.seed);
                return (None, Ok(()));
            }
            "advance" => {
                seam::advance(q.nanoseconds);
                return (None, Ok(()));
            }
            "arm" => return (None, seam::arm(&q.point, q.kill, q.skip).map_err(Error::other)),
            "disarm" => {
                return (
                    Some(Value::map(vec![("reached".into(), Value::Bool(seam::disarm()))])),
                    Ok(()),
                );
            }
            op if op.starts_with("wf_") => return (None, self.workflow(q)),
            "open" => {
                if self.node.is_some() || self.store.is_some() {
                    return (None, Err(Error::other("dost-conform: a node or store is already open")));
                }
                let what = if q.segmented { "OpenSegmented" } else { "Open" };
                return (None, Err(Error::not_ported(what, if q.segmented { 5 } else { 6 })));
            }
            "inspect" => return (None, Err(Error::not_ported("InspectJournal", 8))),
            "repair" => return (None, Err(Error::not_ported("RepairJournal", 8))),
            "upgrade" => return (None, Err(Error::not_ported("UpgradeJournal", 8))),
            _ => {}
        }
        if self.node.is_none() {
            return (None, Err(Error::other("dost-conform: no node is open")));
        }
        let row = match q.op.as_str() {
            "close" | "publish" | "publish_batch" | "read" | "read_batch" | "position" | "commit_batch" => 6,
            "retain" => 7,
            "backup" | "migrate" => 8,
            "verify" => 5,
            _ => return (None, Err(Error::other(format!("dost-conform: unknown op {}", q.op)))),
        };
        (None, Err(Error::not_ported(&q.op, row)))
    }
}

impl Driver {
    /// One `wf_` request. Mirrors `cmd/dost-conform/workflow.go`.
    fn workflow(&mut self, q: &Request) -> Result<(), Error> {
        if q.op == "wf_open" {
            if self.store.is_some() || self.node.is_some() {
                return Err(Error::other("dost-conform: a node or store is already open"));
            }
            return Err(Error::not_ported("workflow.Open", 10));
        }
        if self.store.is_none() {
            return Err(Error::other("dost-conform: no workflow store is open"));
        }
        match q.op.as_str() {
            "wf_close" | "wf_register" | "wf_start_once" | "wf_poll" | "wf_finish" | "wf_signal" | "wf_history" => {
                Err(Error::not_ported(&q.op, 10))
            }
            // Go quotes the name here with strconv.Quote; for the names a test sends that is the same text.
            _ => Err(Error::other(format!("dost-conform: unknown op {:?}", q.op))),
        }
    }
}

/// Serves requests until end of input. An error here ends the driver with status 1, as in Go: input that is not
/// JSON, a field the driver does not know, or a value of the wrong type. A request needs no newline after it: it is
/// answered when its value is complete, which for an object is at its closing brace.
pub fn serve(input: &mut impl BufRead, output: &mut impl Write) -> Result<(), String> {
    let mut driver = Driver::default();
    let mut pending: Vec<u8> = Vec::new();
    let mut at_eof = false;
    loop {
        let parsed = match gojson::parse_prefix(&pending, at_eof) {
            Ok(done) => Some(done),
            Err(ParseError::Incomplete) if !at_eof => None,
            Err(ParseError::Incomplete) if pending.iter().all(u8::is_ascii_whitespace) => return Ok(()),
            Err(ParseError::Incomplete) => return Err("unexpected EOF".into()),
            Err(e) => return Err(e.to_string()),
        };
        let Some((value, used)) = parsed else {
            // Whatever has arrived, not a whole line: a request is answered as soon as its value is complete.
            let arrived = input.fill_buf().map_err(|e| e.to_string())?;
            let count = arrived.len();
            pending.extend_from_slice(arrived);
            input.consume(count);
            at_eof = count == 0;
            continue;
        };
        pending.drain(..used);
        let request = decode(&value)?;
        let (result, outcome) = driver.run(&request);
        output
            .write_all(&reply(result, outcome.as_ref().err()))
            .and_then(|_| output.flush())
            .map_err(|e| e.to_string())?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dostoevsky_core::errors::Kind;

    fn served(input: &str) -> (String, Result<(), String>) {
        let mut out = Vec::new();
        let status = serve(&mut input.as_bytes(), &mut out);
        (String::from_utf8(out).unwrap(), status)
    }

    #[test]
    fn requests_decode_as_in_go() {
        let v = gojson::parse(br#"{"op":"publish","topic":"t","Partition":3,"value":"aGk=","value":"eW8=","key":null,"items":[{"topic":"a","value":null}],"topics":{"a":1,"b":2,"a":4},"nodes":["n1"],"floors":[{"topic":"a","partition":1,"floor":9}]}"#).unwrap();
        let q = decode(&v).unwrap();
        assert_eq!((q.op.as_str(), q.topic.as_str(), q.partition), ("publish", "t", 3));
        assert_eq!(
            q.value.get(),
            Some(&b"yo"[..]),
            "base64 text replaces what an earlier key gave"
        );
        assert_eq!(q.key, "");
        assert_eq!(
            q.items.get(),
            Some(
                &[Message {
                    topic: "a".into(),
                    ..Default::default()
                }][..]
            )
        );
        assert_eq!(q.topics, Some(vec![("a".into(), 4), ("b".into(), 2)]));
        assert_eq!(
            q.floors.get(),
            Some(
                &[Floor {
                    topic: "a".into(),
                    partition: 1,
                    floor: 9
                }][..]
            )
        );
        let w = decode(&gojson::parse(br#"{"op":"wf_register","name":"n","version":2,"steps":[{"kind":"activity","name":"a","attempts":3,"backoff_ms":10}],"attrs":{"k":"v"},"input":"e30="}"#).unwrap()).unwrap();
        assert_eq!(
            w.steps.get(),
            Some(
                &[WorkflowStep {
                    kind: "activity".into(),
                    name: "a".into(),
                    attempts: 3,
                    backoff_ms: 10
                }][..]
            )
        );
        assert_eq!(
            (w.attrs, w.input.get()),
            (Some(vec![("k".into(), "v".into())]), Some(&b"{}"[..]))
        );
        let f =
            decode(&gojson::parse(br#"{"op":"fix_inputs","start":-5,"step":7,"seed":18446744073709551615}"#).unwrap())
                .unwrap();
        assert_eq!((f.start, f.step, f.seed), (-5, 7, u64::MAX));
        for bad in [
            r#"{"seed":"c2VlZA=="}"#,
            r#"{"seed":-1}"#,
            r#"{"seed":18446744073709551616}"#,
            r#"{"skip":1.5}"#,
            r#"{"kill":1}"#,
            r#"{"steps":[{"nope":1}]}"#,
        ] {
            assert!(decode(&gojson::parse(bad.as_bytes()).unwrap()).is_err(), "{bad}");
        }
        // What Go's binding accepts beyond the plain case. The Go driver's own results for several thousand such
        // requests are replayed in tests/go_vectors.rs; these are the four kinds by name.
        let bound = |text: &str| decode(&gojson::parse(text.as_bytes()).unwrap()).unwrap();
        assert_eq!(
            bound(r#"{"value":[104,105]}"#).value.get(),
            Some(&b"hi"[..]),
            "bytes as numbers"
        );
        let folded = bound("{\"\u{17f}kip\":3,\"\u{212a}ill\":true,\"OP\":\"x\"}");
        assert_eq!(
            (folded.skip, folded.kill, folded.op.as_str()),
            (3, true, "x"),
            "folded names"
        );
        assert_eq!(bound("null"), Request::default(), "null for a request");
        assert_eq!(
            bound(r#"{"nodes":[null,"b"]}"#).nodes.get(),
            Some(&[String::new(), "b".into()][..])
        );
        let again = bound(r#"{"nodes":["a","b","c"],"nodes":["z"],"nodes":[null,null]}"#);
        assert_eq!(
            again.nodes.get(),
            Some(&["z".to_string(), "b".into()][..]),
            "a repeated key binds onto what the earlier one left, as in Go"
        );
        for bad in [
            r#"{"op":1}"#,
            r#"{"surprise":1}"#,
            r#"{"offset":1.0}"#,
            r#"{"offset":1e2}"#,
            r#"{"offset":"1"}"#,
            r#"{"value":"!"}"#,
            r#"{"items":[{"nope":1}]}"#,
            r#"{"value":[256]}"#,
            r#"{"value":[-0]}"#,
            "{\"o\u{440}\":\"x\"}",
            r#"[]"#,
        ] {
            assert!(decode(&gojson::parse(bad.as_bytes()).unwrap()).is_err(), "{bad}");
        }
    }

    #[test]
    fn replies_are_the_go_driver_s_bytes() {
        assert_eq!(reply(None, None), b"{\"ok\":true}\n");
        let record = Value::Object(vec![
            ("topic".into(), Value::text("t")),
            ("partition".into(), Value::int(0)),
            ("offset".into(), Value::int(7)),
            ("key".into(), Value::text("")),
            ("value".into(), Value::bytes(Some(b"hi"))),
            ("id".into(), Value::text("")),
            ("time".into(), Value::text("1790000000000000000")),
        ]);
        assert_eq!(
            String::from_utf8(reply(Some(record), None)).unwrap(),
            "{\"ok\":true,\"result\":{\"topic\":\"t\",\"partition\":0,\"offset\":7,\"key\":\"\",\"value\":\"aGk=\",\"id\":\"\",\"time\":\"1790000000000000000\"}}\n"
        );
        let refused = Error::new(Kind::LaterFrames, "frames follow the <damage>");
        assert_eq!(
            String::from_utf8(reply(Some(Value::map(vec![("size".into(), Value::int(1))])), Some(&refused))).unwrap(),
            "{\"ok\":false,\"error\":\"later_frames\",\"message\":\"frames follow the \\u003cdamage\\u003e\",\"result\":{\"size\":1}}\n"
        );
    }

    #[test]
    fn serves_until_end_of_input_and_keeps_the_order_of_checks() {
        let (out, status) = served(
            "{\"op\":\"close\"}\n{\"op\":\"nonsense\"}\n{\"op\":\"open\",\n \"dir\":\"d\",\"segmented\":true}\n\n",
        );
        assert_eq!(status, Ok(()));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[0],
            r#"{"ok":false,"error":"other","message":"dost-conform: no node is open"}"#
        );
        assert_eq!(
            lines[1], lines[0],
            "an unknown request is refused for the missing node first, as in Go"
        );
        assert_eq!(
            lines[2],
            r#"{"ok":false,"error":"other","message":"dostoevsky: OpenSegmented is not ported yet (row 5)"}"#
        );
        assert_eq!(served("").1, Ok(()));
        let (out, status) = served(concat!(
            "{\"op\":\"wf_poll\",\"queue\":\"q\"}\n{\"op\":\"wf_open\",\"dir\":\"w\"}\n",
            "{\"op\":\"fix_inputs\",\"start\":1,\"step\":2,\"seed\":3}\n{\"op\":\"advance\",\"nanoseconds\":5}\n",
            "{\"op\":\"arm\",\"point\":\"segment.write\",\"kill\":true,\"skip\":-1}\n{\"op\":\"disarm\"}\n",
            "{\"op\":\"arm\",\"point\":\"p\",\"kill\":false,\"skip\":1}\n"
        ));
        assert_eq!(status, Ok(()));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[0],
            r#"{"ok":false,"error":"other","message":"dost-conform: no workflow store is open"}"#
        );
        assert_eq!(
            lines[1],
            r#"{"ok":false,"error":"other","message":"dostoevsky: workflow.Open is not ported yet (row 10)"}"#
        );
        assert_eq!(&lines[2..5], [r#"{"ok":true}"#, r#"{"ok":true}"#, r#"{"ok":true}"#]);
        assert_eq!(
            lines[5], r#"{"ok":true,"result":{"reached":false}}"#,
            "armed, never reached"
        );
        assert_eq!(
            lines[6],
            r#"{"ok":false,"error":"other","message":"seam: no point is named \"p\""}"#
        );
        assert_eq!(
            (seam::now_nanos(), seam::now_nanos()),
            (6, 8),
            "start 1, advanced by 5, step 2"
        );
        seam::restore();
        assert!(served("{\"op\":\"close\"").1.is_err());
        assert!(served("{\"op\":\"close\",\"surprise\":1}\n").1.is_err());
        assert!(served("not json\n").1.is_err());
    }
}
