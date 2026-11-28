//! The command-line tools and the conformance driver.
//!
//! Phase 1 mirrors the Go engine module by module. Each module below names the Go file it mirrors and the
//! row of the port in which it is filled in. See `rust/PORT-NOTES.md` for the full map.

pub mod conform;
