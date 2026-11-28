//! A point armed to kill: the line it writes and the way the process dies.

#![cfg(unix)]

use dostoevsky_core::seam;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

const CHILD: &str = "DOST_SEAM_KILL_POINT_CHILD";

/// Runs only in the process the next test starts: it reaches an armed kill point and must not return from it.
#[test]
fn child_reaches_the_kill_point() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    seam::arm("segment.sync", true, 2).unwrap();
    assert_eq!(seam::at("segment.sync"), Ok(()));
    assert_eq!(seam::at("segment.write"), Ok(()));
    assert_eq!(seam::at("segment.sync"), Ok(()));
    let _ = seam::at("segment.sync");
    eprintln!("the process outlived its kill point");
    std::process::exit(3);
}

#[test]
fn a_kill_point_writes_its_line_and_the_process_dies_by_sigkill() {
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_reaches_the_kill_point", "--test-threads=1"])
        .env(CHILD, "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(child.status.signal(), Some(9), "{:?}", child.status);
    assert_eq!(
        String::from_utf8_lossy(&child.stderr),
        "seam: killing at segment.sync, pass 3\n",
        "exactly this line and nothing else, with pass = skip + 1"
    );
}
