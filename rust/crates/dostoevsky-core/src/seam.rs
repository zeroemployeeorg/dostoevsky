//! The test seam: an injected clock and an injected random source, and named points at which a test can stop the
//! engine. Mirrors `internal/seam` of the Go engine.
//!
//! Unset, the engine reads the system clock and the system's random source, as it must in production. Fixed, two runs
//! and two implementations given the same requests one at a time write the same bytes:
//!
//! - **Clock:** the k-th reading after [`fix`] (k from 0) is `start + k * step` nanoseconds since the Unix epoch,
//!   plus whatever [`advance`] has added.
//! - **Random bytes:** each request for n bytes takes SHA-256(seed || counter) blocks, seed and counter as 8 bytes
//!   little-endian, the counter starting at 0 after [`fix`] and going up by one per block, and keeps the first n
//!   bytes. What is left of the last block is not used again.
//! - **Same moments, same count:** this engine reads the clock and the random source at the same points in each
//!   operation, and as often, as the Go engine. Each reading site names the Go line it mirrors.
//! - **Write order:** where the Go engine writes partitions and consumer groups it holds in maps, it writes
//!   partitions ordered by topic and then number, and groups by name; checkpoints order partitions by their stored
//!   identifier. This engine writes in the same orders.
//!
//! [`fix`] is not safe while a node or store is open: call it before opening anything.
//!
//! The clock and the random source are checked against vectors written by the Go package (`rust/testdata`). The
//! named points ([`POINTS`], [`arm`], [`at`], [`disarm`]) mirror Go's `internal/seam/points.go`.

use dostoevsky_format::sha256;
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixed {
    next: i64,
    step: i64,
    seed: u64,
    count: u64,
}

static FIXED: Mutex<Option<Fixed>> = Mutex::new(None);

fn fixed() -> std::sync::MutexGuard<'static, Option<Fixed>> {
    FIXED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Fixes the clock and the random source for this process. Mirrors `seam.Fix`.
pub fn fix(start: i64, step: i64, seed: u64) {
    *fixed() = Some(Fixed {
        next: start,
        step,
        seed,
        count: 0,
    });
}

/// Back to the system clock and the system's random source. Mirrors `seam.Restore`.
pub fn restore() {
    *fixed() = None;
}

pub fn is_fixed() -> bool {
    fixed().is_some()
}

/// Moves a fixed clock by `nanoseconds`. Does nothing when the clock is not fixed. Mirrors `seam.Advance`.
pub fn advance(nanoseconds: i64) {
    if let Some(f) = fixed().as_mut() {
        f.next = f.next.wrapping_add(nanoseconds);
    }
}

/// Nanoseconds since the Unix epoch. Mirrors `seam.Now`.
pub fn now_nanos() -> i64 {
    if let Some(f) = fixed().as_mut() {
        let t = f.next;
        f.next = f.next.wrapping_add(f.step);
        return t;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i64)
}

/// Fills `out` with random bytes. Mirrors `seam.Random`.
pub fn random_bytes(out: &mut [u8]) -> std::io::Result<()> {
    if let Some(f) = fixed().as_mut() {
        let mut input = [0u8; 16];
        input[..8].copy_from_slice(&f.seed.to_le_bytes());
        for chunk in out.chunks_mut(32) {
            input[8..].copy_from_slice(&f.count.to_le_bytes());
            f.count += 1;
            chunk.copy_from_slice(&sha256::digest(&input)[..chunk.len()]);
        }
        return Ok(());
    }
    system_random(out)
}

#[cfg(unix)]
fn system_random(out: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(out)
}

#[cfg(not(unix))]
fn system_random(_out: &mut [u8]) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no system random source on this platform yet",
    ))
}

// ----- named points -----

/// The named places where a test can make the engine fail or die. Mirrors `Points` in Go's `internal/seam/points.go`:
/// the same names at the same boundaries. A point named for an operation is reached before that operation, unless its
/// name says when.
pub const POINTS: [&str; 18] = [
    // A legacy journal append: before the write, and between the write and its sync.
    "journal.write",
    "journal.sync",
    // A segment append, likewise.
    "segment.write",
    "segment.sync",
    // Before the new range file replaces the old one.
    "range.publish",
    // A directory sync made by the engine's own directory-sync function: the directory chain at open, segmented and
    // workflow initialization and the cleanup of an abandoned one, a staged backup and its parent, after the range
    // file is replaced, after retention removes a segment file, and after a workflow metadata file is replaced. NOT
    // counted: the syncs the segment layer makes when it creates a segment or publishes an index or filter side file,
    // the one made when a legacy journal file is created, those of repair and upgrade, and legacy retention's, which
    // is retention.dirsync.
    "dirsync",
    // Legacy retention: each frame of the replacement, its sync, the rename, the directory sync and the release of
    // the replaced journal.
    "retention.write",
    "retention.sync",
    "retention.rename",
    "retention.dirsync",
    "retention.truncate",
    // Backup and migration: before the copy, before the copy is published, before the parent directory is synced.
    "backup.write",
    "backup.rename",
    "backup.syncparent",
    // A workflow commit: before anything is written, once the call has passed the closed, fenced and cancellation
    // checks and before its items are examined; part-way through writing the group (on the pass where it fires, and
    // only then, half the bytes are written first; it counts segment appends, so arm it only where a workflow store
    // is the thing writing); after the segment is synced and before the witness is replaced; after the witness is
    // replaced and before any caller is answered.
    "workflow.before_append",
    "workflow.mid_append",
    "workflow.after_segment_sync_before_witness",
    "workflow.after_witness_before_reply",
];

/// The error a point armed to fail returns. Go's `ErrInjected`, with the point's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Injected(pub String);

impl std::fmt::Display for Injected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "injected failure at {}", self.0)
    }
}

impl std::error::Error for Injected {}

struct Armed {
    name: String,
    kill: bool,
    /// Counts down by one per pass; the pass that takes it to -1 fires.
    skip: i64,
    /// The pass that fires: the armed `skip` plus one.
    pass: i64,
    reached: bool,
}

static ARMED: Mutex<Option<Armed>> = Mutex::new(None);

fn armed() -> std::sync::MutexGuard<'static, Option<Armed>> {
    ARMED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Makes the named point fail, or kill the process, once: the first time it is reached after `skip` earlier times.
/// Every later pass goes through, and with a negative `skip` the point never fires. A name that is not in [`POINTS`]
/// is refused. One point is armed at a time, and an armed point stays until [`disarm`] has reported it: arming again
/// before that is refused, so that no test replaces a point it never checked.
///
/// Mirrors `seam.Arm`, with the rule that an unchecked point refuses a second `arm`.
pub fn arm(name: &str, kill: bool, skip: i64) -> Result<(), String> {
    if !POINTS.contains(&name) {
        return Err(format!("seam: no point is named {name:?}"));
    }
    let mut armed = armed();
    if let Some(a) = armed.as_ref() {
        return Err(format!(
            "seam: point {:?} is armed and disarm has not reported it",
            a.name
        ));
    }
    *armed = Some(Armed {
        name: name.to_string(),
        kill,
        skip,
        pass: skip.wrapping_add(1),
        reached: false,
    });
    Ok(())
}

/// The one line a kill point writes to standard error before the process dies. A test runner accepts a death only
/// with this line, inside the request it marked as the one that dies.
pub fn kill_line(name: &str, pass: i64) -> String {
    format!("seam: killing at {name}, pass {pass}\n")
}

/// Removes the armed point and reports whether it fired. Mirrors `seam.Disarm`.
pub fn disarm() -> bool {
    armed().take().is_some_and(|a| a.reached)
}

/// Counts one pass of the named point and reports whether this is the pass that fires. A point that has to do work
/// before it fires calls `due`, does the work and then calls [`fire`]; every other point uses [`at`]. Mirrors
/// `seam.Due`.
pub fn due(name: &str) -> bool {
    match armed().as_mut().filter(|a| a.name == name) {
        Some(a) => {
            a.skip = a.skip.wrapping_sub(1);
            a.skip == -1
        }
        None => false,
    }
}

/// Fails or kills at the armed point, after [`due`] has said that this pass is the one. Mirrors `seam.Fire`.
pub fn fire(name: &str) -> Result<(), Injected> {
    let kill = match armed().as_mut().filter(|a| a.name == name) {
        Some(a) => {
            a.reached = true;
            a.kill.then_some(a.pass)
        }
        None => return Ok(()),
    };
    if let Some(pass) = kill {
        // One write of the whole line, so that nothing can come between its parts. A failed write changes nothing:
        // the process dies either way, and the runner then rejects the death.
        let _ = std::io::stderr().write_all(kill_line(name, pass).as_bytes());
        kill_this_process();
    }
    Err(Injected(name.to_string()))
}

/// Called by the engine at a point. `Ok` unless a test armed this point and this is its pass. Mirrors `seam.At`.
pub fn at(name: &str) -> Result<(), Injected> {
    if !due(name) {
        return Ok(());
    }
    fire(name)
}

#[cfg(unix)]
fn kill_this_process() -> ! {
    extern "C" {
        fn getpid() -> i32;
        fn kill(pid: i32, signal: i32) -> i32;
    }
    const SIGKILL: i32 = 9;
    // SAFETY: both are async-signal-safe C library calls with plain integer arguments.
    unsafe {
        kill(getpid(), SIGKILL);
    }
    loop {
        std::thread::park();
    }
}

#[cfg(not(unix))]
fn kill_this_process() -> ! {
    std::process::abort()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The seam is one per process, so its tests run as one.
    #[test]
    fn the_contract_as_the_go_package_keeps_it() {
        restore();
        let before = now_nanos();
        assert!(before > 1_700_000_000_000_000_000 && now_nanos() >= before);
        let (mut a, mut b) = ([0u8; 16], [0u8; 16]);
        random_bytes(&mut a).unwrap();
        random_bytes(&mut b).unwrap();
        assert_ne!(a, b);
        advance(5); // nothing to move

        // Every step the Go program took, and what Go returned.
        let steps = include_str!("../../../testdata/seam-go.txt");
        let (mut readings, mut requests) = (0, 0);
        for line in steps.lines().filter(|l| !l.starts_with('#')) {
            let f: Vec<&str> = line.split(' ').collect();
            match f[0] {
                "fix" => fix(f[1].parse().unwrap(), f[2].parse().unwrap(), f[3].parse().unwrap()),
                "advance" => advance(f[1].parse().unwrap()),
                "now" => {
                    assert_eq!(now_nanos(), f[1].parse::<i64>().unwrap(), "{line}");
                    readings += 1;
                }
                "random" => {
                    let mut got = vec![0u8; f[1].parse().unwrap()];
                    random_bytes(&mut got).unwrap();
                    assert_eq!(sha256::hex(&got), *f.get(2).unwrap_or(&""), "{line}");
                    requests += 1;
                }
                other => panic!("unknown step {other}"),
            }
        }
        assert_eq!((readings, requests), (20, 32));
        assert!(is_fixed());
        restore();
        assert!(!is_fixed());

        // Named points.
        assert_eq!(at("segment.write"), Ok(()), "nothing is armed");
        assert!(!disarm());
        assert_eq!(
            arm("no.such.point", false, 0),
            Err("seam: no point is named \"no.such.point\"".to_string())
        );
        arm("segment.write", false, 2).unwrap();
        assert_eq!(at("segment.sync"), Ok(()), "another point");
        let passes = [
            at("segment.write"),
            at("segment.write"),
            at("segment.write"),
            at("segment.write"),
        ];
        assert_eq!(
            passes,
            [Ok(()), Ok(()), Err(Injected("segment.write".into())), Ok(())],
            "it fires once, on pass skip + 1"
        );
        assert!(disarm(), "it fired");
        assert!(!disarm());
        arm("dirsync", false, 1).unwrap();
        assert_eq!(at("dirsync"), Ok(()));
        assert_eq!(
            arm("segment.write", false, 0),
            Err("seam: point \"dirsync\" is armed and disarm has not reported it".to_string()),
            "an unchecked point refuses a second arm"
        );
        assert_eq!(
            arm("dirsync", false, 0),
            Err("seam: point \"dirsync\" is armed and disarm has not reported it".to_string()),
            "also for the same name"
        );
        assert_eq!(at("segment.write"), Ok(()), "the refused arm changed nothing");
        assert!(!disarm(), "reached once, but it had not fired");
        arm("segment.write", false, 0).unwrap();
        assert_eq!(at("segment.write"), Err(Injected("segment.write".into())));
        assert!(
            arm("dirsync", false, 0).is_err(),
            "a point that fired is still unchecked"
        );
        assert!(disarm());
        arm("dirsync", true, -1).unwrap();
        assert_eq!(
            [at("dirsync"), at("dirsync"), at("dirsync")],
            [Ok(()), Ok(()), Ok(())],
            "a negative skip never fires"
        );
        assert!(!disarm());
        // The two halves, for a point that does work before it fires.
        arm("workflow.mid_append", false, 0).unwrap();
        assert!(due("workflow.mid_append"));
        assert_eq!(fire("workflow.mid_append"), Err(Injected("workflow.mid_append".into())));
        assert!(!due("workflow.mid_append"));
        assert!(disarm());
        assert_eq!(kill_line("segment.sync", 4), "seam: killing at segment.sync, pass 4\n");
        assert_eq!(POINTS.len(), 18);
    }
}
