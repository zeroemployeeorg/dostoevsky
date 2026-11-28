//! On-disk encodings: frames, checkpoints, sidecars, the journal encoding, and the Go-compatible JSON reader and writer. No I/O.
//!
//! Phase 1 mirrors the Go engine module by module. Each module below names the Go file it mirrors and the
//! row of the port in which it is filled in. See `rust/PORT-NOTES.md` for the full map.

pub mod base64;
pub mod crc32c;
pub mod gojson;
pub mod gotime;
pub mod sha256;

pub mod checkpoint3;
pub mod frame3;
pub mod idfilter;
pub mod recordscan;
pub mod segindex;
