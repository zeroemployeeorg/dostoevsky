//! Checkpoint frames of the segmented log. Mirrors the format half of `checkpoint3.go`; the replayer in that file
//! applies frames to the log state and is ported with it, in `dostoevsky-core`.
//!
//! A checkpoint is the whole durable state written as a sequence of frames:
//!
//! ```text
//! cp-begin   id u64 | segment id u64 | topics u32 | partitions u32 | groups u32
//! config     the manifest (JSON of the configuration)
//! topic-define x topics     (frame type 4, as for records)
//! cp-partition x partitions partition id u32 | base u64 | next u64
//! cp-group x groups         partition id u32 | group (u16 length, bytes) | next u64 | state
//! cp-end     id u64 | topics u32 | partitions u32 | groups u32
//! ```
//!
//! It is valid only if cp-end is present with the same id and counts as cp-begin and every frame between them checks
//! out. Within it, partition ids, topic definitions and groups are in one canonical ascending order without repeats
//! (the writer produces that order, the reader requires it). A crash in the middle of writing one leaves an
//! incomplete checkpoint, which replay ignores.

use crate::frame3::{
    append_frame3, append_topic_def3, parse_topic_def3, read_frame3, Frame3Error, TopicDef3, F3_TOPIC_DEF,
};
use std::fmt;

pub const F3_CP_BEGIN: u8 = 3;
pub const F3_CONFIG: u8 = 6;
pub const F3_CP_PARTITION: u8 = 7;
pub const F3_CP_GROUP: u8 = 8;
pub const F3_CP_END: u8 = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointError {
    /// The data ended inside a checkpoint (the usual result of a crash). Go's `errCheckpointTorn`.
    Torn(String),
    /// The frames are there but are not a valid checkpoint. Go's `errCheckpointInvalid`.
    Invalid(String),
    /// Go's `ErrLimit`.
    Limit,
}

impl fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CheckpointError::Torn(what) => write!(f, "checkpoint: incomplete: {what}"),
            CheckpointError::Invalid(what) => write!(f, "checkpoint: invalid: {what}"),
            CheckpointError::Limit => f.write_str("size limit exceeded"),
        }
    }
}

impl std::error::Error for CheckpointError {}

fn cp_invalid<T>(what: String) -> Result<T, CheckpointError> {
    Err(CheckpointError::Invalid(what))
}

/// A topic definition that owns its name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CpTopic3 {
    pub partition: u32,
    pub number: u32,
    pub topic: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpPartition3 {
    pub partition: u32,
    pub base: u64,
    pub next: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CpGroup3 {
    pub partition: u32,
    pub group: Vec<u8>,
    pub next: u64,
    /// `None` for an empty state. Go's `cloneState` turns empty into nil, so that two states that differ only in nil
    /// against empty compare equal.
    pub state: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Checkpoint3 {
    pub id: u64,
    pub segment: u64,
    pub config: Vec<u8>,
    pub topics: Vec<CpTopic3>,
    pub partitions: Vec<CpPartition3>,
    pub groups: Vec<CpGroup3>,
}

fn cp_counts(b: &mut Vec<u8>, topics: usize, partitions: usize, groups: usize) {
    b.extend_from_slice(&(topics as u32).to_le_bytes());
    b.extend_from_slice(&(partitions as u32).to_le_bytes());
    b.extend_from_slice(&(groups as u32).to_le_bytes());
}

fn limit(e: Frame3Error) -> CheckpointError {
    match e {
        Frame3Error::Limit => CheckpointError::Limit,
        other => CheckpointError::Invalid(other.to_string()),
    }
}

/// Appends the frames of one checkpoint to `dst`. On an error `dst` is left unchanged in length.
pub fn append_checkpoint3(dst: &mut Vec<u8>, cp: &Checkpoint3) -> Result<(), CheckpointError> {
    if cp.topics.len() as u64 > u32::MAX as u64
        || cp.partitions.len() as u64 > u32::MAX as u64
        || cp.groups.len() as u64 > u32::MAX as u64
    {
        return Err(CheckpointError::Limit);
    }
    let start = dst.len();
    let result = append_frames(dst, cp);
    if result.is_err() {
        dst.truncate(start);
    }
    result
}

fn append_frames(dst: &mut Vec<u8>, cp: &Checkpoint3) -> Result<(), CheckpointError> {
    let mut body = Vec::new();
    body.extend_from_slice(&cp.id.to_le_bytes());
    body.extend_from_slice(&cp.segment.to_le_bytes());
    cp_counts(&mut body, cp.topics.len(), cp.partitions.len(), cp.groups.len());
    append_frame3(dst, F3_CP_BEGIN, &body).map_err(limit)?;
    append_frame3(dst, F3_CONFIG, &cp.config).map_err(limit)?;
    for t in &cp.topics {
        append_topic_def3(
            dst,
            &TopicDef3 {
                partition: t.partition,
                number: t.number,
                topic: &t.topic,
            },
        )
        .map_err(limit)?;
    }
    for p in &cp.partitions {
        body.clear();
        body.extend_from_slice(&p.partition.to_le_bytes());
        body.extend_from_slice(&p.base.to_le_bytes());
        body.extend_from_slice(&p.next.to_le_bytes());
        append_frame3(dst, F3_CP_PARTITION, &body).map_err(limit)?;
    }
    for g in &cp.groups {
        if g.group.len() > u16::MAX as usize {
            return Err(CheckpointError::Limit);
        }
        body.clear();
        body.extend_from_slice(&g.partition.to_le_bytes());
        body.extend_from_slice(&(g.group.len() as u16).to_le_bytes());
        body.extend_from_slice(&g.group);
        body.extend_from_slice(&g.next.to_le_bytes());
        body.extend_from_slice(g.state.as_deref().unwrap_or_default());
        append_frame3(dst, F3_CP_GROUP, &body).map_err(limit)?;
    }
    body.clear();
    body.extend_from_slice(&cp.id.to_le_bytes());
    cp_counts(&mut body, cp.topics.len(), cp.partitions.len(), cp.groups.len());
    append_frame3(dst, F3_CP_END, &body).map_err(limit)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("four bytes"))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("eight bytes"))
}

fn parse_cp_counts(b: &[u8]) -> (usize, usize, usize) {
    (u32_at(b, 0) as usize, u32_at(b, 4) as usize, u32_at(b, 8) as usize)
}

/// Collects the frames of one checkpoint as they are read, enforcing the order begin, config, topic-defines,
/// partitions, groups, end, with the counts cp-begin gave.
#[derive(Debug, Default)]
pub struct CpBuilder {
    pub cp: Checkpoint3,
    want_topics: usize,
    want_parts: usize,
    want_groups: usize,
    saw_config: bool,
}

pub fn begin_checkpoint3(body: &[u8]) -> Result<CpBuilder, CheckpointError> {
    if body.len() != 8 + 8 + 12 {
        return cp_invalid(format!("cp-begin of {} bytes", body.len()));
    }
    let mut b = CpBuilder::default();
    b.cp.id = u64_at(body, 0);
    b.cp.segment = u64_at(body, 8);
    (b.want_topics, b.want_parts, b.want_groups) = parse_cp_counts(&body[16..]);
    Ok(b)
}

impl CpBuilder {
    /// Takes the next frame of the checkpoint. The result is true after a valid cp-end.
    pub fn add(&mut self, typ: u8, body: &[u8]) -> Result<bool, CheckpointError> {
        if !self.saw_config {
            if typ != F3_CONFIG {
                return cp_invalid(format!("expected the config frame, got type {typ}"));
            }
            self.cp.config = body.to_vec();
            self.saw_config = true;
        } else if self.cp.topics.len() < self.want_topics {
            if typ != F3_TOPIC_DEF {
                return cp_invalid(format!("expected a topic definition, got type {typ}"));
            }
            let d = match parse_topic_def3(body) {
                Ok(d) => d,
                Err(e) => return cp_invalid(e.to_string()),
            };
            if let Some(last) = self.cp.topics.last() {
                if d.partition <= last.partition {
                    return cp_invalid(format!(
                        "topic definitions are not in ascending partition id order (id {} after {})",
                        d.partition, last.partition
                    ));
                }
            }
            self.cp.topics.push(CpTopic3 {
                partition: d.partition,
                number: d.number,
                topic: d.topic.to_vec(),
            });
        } else if self.cp.partitions.len() < self.want_parts {
            if typ != F3_CP_PARTITION || body.len() != 4 + 8 + 8 {
                return cp_invalid(format!(
                    "expected a partition entry of 20 bytes, got type {typ} of {} bytes",
                    body.len()
                ));
            }
            let pe = CpPartition3 {
                partition: u32_at(body, 0),
                base: u64_at(body, 4),
                next: u64_at(body, 12),
            };
            if let Some(last) = self.cp.partitions.last() {
                if pe.partition <= last.partition {
                    return cp_invalid(format!(
                        "partition entries are not in ascending id order (id {} after {})",
                        pe.partition, last.partition
                    ));
                }
            }
            self.cp.partitions.push(pe);
        } else if self.cp.groups.len() < self.want_groups {
            if typ != F3_CP_GROUP || body.len() < 4 + 2 + 8 {
                return cp_invalid(format!(
                    "expected a group entry, got type {typ} of {} bytes",
                    body.len()
                ));
            }
            let g = u16::from_le_bytes([body[4], body[5]]) as usize;
            if body.len() < 4 + 2 + g + 8 {
                return cp_invalid(format!("group name of {g} bytes exceeds the entry"));
            }
            let state = &body[6 + g + 8..];
            let ge = CpGroup3 {
                partition: u32_at(body, 0),
                group: body[6..6 + g].to_vec(),
                next: u64_at(body, 6 + g),
                state: if state.is_empty() { None } else { Some(state.to_vec()) },
            };
            if let Some(prev) = self.cp.groups.last() {
                if ge.partition < prev.partition || (ge.partition == prev.partition && ge.group <= prev.group) {
                    return cp_invalid(
                        "group entries are not in ascending (partition id, name) order, or repeat".into(),
                    );
                }
            }
            self.cp.groups.push(ge);
        } else {
            if typ != F3_CP_END || body.len() != 8 + 12 {
                return cp_invalid(format!("expected cp-end, got type {typ} of {} bytes", body.len()));
            }
            if u64_at(body, 0) != self.cp.id
                || parse_cp_counts(&body[8..]) != (self.want_topics, self.want_parts, self.want_groups)
            {
                return cp_invalid("cp-end does not match cp-begin".into());
            }
            return Ok(true);
        }
        Ok(false)
    }
}

/// Reads one checkpoint from the start of `b` and returns it with the number of bytes it occupies.
/// [`CheckpointError::Torn`] means `b` ended inside it; [`CheckpointError::Invalid`] means it is not a valid
/// checkpoint (a checksum failure counts as invalid: the caller decides whether damage at the very end of a file is
/// a torn write).
pub fn read_checkpoint3(b: &[u8]) -> Result<(Checkpoint3, usize), CheckpointError> {
    let (typ, body, n) = read_frame3(b).map_err(cp_frame_error)?;
    if typ != F3_CP_BEGIN {
        return cp_invalid(format!("starts with type {typ}, not cp-begin"));
    }
    let mut build = begin_checkpoint3(body)?;
    let mut size = n;
    loop {
        let (typ, body, n) = read_frame3(&b[size..]).map_err(cp_frame_error)?;
        size += n;
        if build.add(typ, body)? {
            return Ok((build.cp, size));
        }
    }
}

pub fn cp_frame_error(e: Frame3Error) -> CheckpointError {
    match e {
        Frame3Error::Short => CheckpointError::Torn(e.to_string()),
        other => CheckpointError::Invalid(other.to_string()),
    }
}
