//! The driver as a process: what a test runner on the other end of its pipes sees.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(20);

#[test]
fn a_request_is_answered_at_its_closing_brace() {
    let mut driver = Command::new(env!("CARGO_BIN_EXE_dost-conform"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut requests = driver.stdin.take().unwrap();
    let (send, replies) = mpsc::channel();
    let mut output = BufReader::new(driver.stdout.take().unwrap());
    let reader = std::thread::spawn(move || loop {
        let mut line = String::new();
        if output.read_line(&mut line).unwrap() == 0 || send.send(line).is_err() {
            return;
        }
    });
    // No newline follows either request, and the input stays open: the reply must come all the same.
    for (request, want) in [
        ("{\"op\":\"disarm\"}", "{\"ok\":true,\"result\":{\"reached\":false}}\n"),
        (
            " {\"op\":\n\"close\"}",
            "{\"ok\":false,\"error\":\"other\",\"message\":\"dost-conform: no node is open\"}\n",
        ),
    ] {
        requests.write_all(request.as_bytes()).unwrap();
        requests.flush().unwrap();
        let reply = replies.recv_timeout(DEADLINE);
        if reply.is_err() {
            driver.kill().unwrap();
        }
        assert_eq!(reply.as_deref(), Ok(want), "{request}");
    }
    drop(requests);
    assert!(driver.wait().unwrap().success());
    reader.join().unwrap();
}

#[test]
fn input_the_go_driver_stops_at_ends_the_driver_with_status_1() {
    for input in [
        "{\"op\":\"close\",\"surprise\":1}\n",
        "[]\n",
        "{\"op\":\"close\"",
        "{\"value\":[256]}\n",
    ] {
        let mut driver = Command::new(env!("CARGO_BIN_EXE_dost-conform"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        driver.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        let done = driver.wait_with_output().unwrap();
        assert_eq!(done.status.code(), Some(1), "{input}");
        assert!(done.stdout.is_empty() && !done.stderr.is_empty(), "{input}");
    }
}
