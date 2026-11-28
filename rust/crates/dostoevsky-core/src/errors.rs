//! The errors a caller can tell apart. Mirrors the sentinel errors of the Go engine (`Err...` in `node.go` and its
//! neighbours) and the code names the conformance driver reports for them.
//!
//! A Go error can wrap more than one sentinel, and the Go driver reports the first match in a fixed order. So an
//! [`Error`] carries every kind it wraps, and [`Error::code`] applies the same order.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    // The order is the order the Go driver tests in: an error that wraps another comes first.
    JournalFull,
    Limit,
    Invalid,
    Owner,
    Closed,
    Conflict,
    Expired,
    Empty,
    CorruptLog,
    IncompleteBackup,
    BackupNotDurable,
    UpgradeRequired,
    SegmentedJournal,
    LaterFrames,
    NotRepairable,
    /// Go's `fs.ErrExist`.
    Exists,
}

impl Kind {
    pub const ALL: [Kind; 16] = [
        Kind::JournalFull,
        Kind::Limit,
        Kind::Invalid,
        Kind::Owner,
        Kind::Closed,
        Kind::Conflict,
        Kind::Expired,
        Kind::Empty,
        Kind::CorruptLog,
        Kind::IncompleteBackup,
        Kind::BackupNotDurable,
        Kind::UpgradeRequired,
        Kind::SegmentedJournal,
        Kind::LaterFrames,
        Kind::NotRepairable,
        Kind::Exists,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Kind::JournalFull => "journal_full",
            Kind::Limit => "limit",
            Kind::Invalid => "invalid",
            Kind::Owner => "owner",
            Kind::Closed => "closed",
            Kind::Conflict => "conflict",
            Kind::Expired => "expired",
            Kind::Empty => "empty",
            Kind::CorruptLog => "corrupt_log",
            Kind::IncompleteBackup => "incomplete_backup",
            Kind::BackupNotDurable => "backup_not_durable",
            Kind::UpgradeRequired => "upgrade_required",
            Kind::SegmentedJournal => "segmented_journal",
            Kind::LaterFrames => "later_frames",
            Kind::NotRepairable => "not_repairable",
            Kind::Exists => "exists",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kinds: Vec<Kind>,
    message: String,
}

impl Error {
    /// An error with no kind a caller can test for. The driver reports it as `other`.
    pub fn other(message: impl Into<String>) -> Error {
        Error {
            kinds: Vec::new(),
            message: message.into(),
        }
    }

    pub fn new(kind: Kind, message: impl Into<String>) -> Error {
        Error {
            kinds: vec![kind],
            message: message.into(),
        }
    }

    /// The same error, also wrapping `kind`.
    pub fn also(mut self, kind: Kind) -> Error {
        if !self.kinds.contains(&kind) {
            self.kinds.push(kind);
        }
        self
    }

    /// Go's `errors.Is` against one sentinel.
    pub fn is(&self, kind: Kind) -> bool {
        self.kinds.contains(&kind)
    }

    /// The name the conformance driver reports.
    pub fn code(&self) -> &'static str {
        Kind::ALL.iter().find(|k| self.is(**k)).map_or("other", |k| k.code())
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// What every engine operation returns until its row is ported.
    pub fn not_ported(operation: &str, row: u8) -> Error {
        Error::other(format!("dostoevsky: {operation} is not ported yet (row {row})"))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    /// The names in the Go driver's `codes` table, in its order, read from the Go source.
    fn go_driver_codes() -> Vec<String> {
        let source = include_str!("../../../../cmd/dost-conform/main.go");
        let table = source.split("var codes = []struct {").nth(1).expect("the codes table");
        let table = &table[..table.find("\n}\n").expect("the end of the codes table")];
        table
            .lines()
            .filter_map(|line| line.trim().strip_prefix("{\""))
            .map(|rest| rest[..rest.find('"').unwrap()].to_string())
            .collect()
    }

    #[test]
    fn the_kinds_are_the_go_driver_s_codes_in_its_order() {
        let go = go_driver_codes();
        assert_eq!(go.len(), 19, "{go:?}");
        // The driver's table ends with the three workflow errors, which arrive with the workflow store.
        assert_eq!(&go[16..], ["start_conflict", "stale", "missing"]);
        let rust: Vec<&str> = Kind::ALL.iter().map(|k| k.code()).collect();
        assert_eq!(rust, go[..16], "Kind::ALL against cmd/dost-conform/main.go");
    }

    #[test]
    fn the_first_kind_in_the_driver_order_is_reported() {
        assert_eq!(Error::other("x").code(), "other");
        // Every pair, built in both orders: the kind that comes first in the driver's table is the one reported.
        for (i, first) in Kind::ALL.iter().enumerate() {
            assert_eq!(Error::new(*first, "x").code(), first.code());
            for later in &Kind::ALL[i + 1..] {
                assert_eq!(Error::new(*first, "x").also(*later).code(), first.code());
                assert_eq!(Error::new(*later, "x").also(*first).code(), first.code());
                assert_ne!(first.code(), later.code());
            }
        }
        // And by name, for the pairs the Go engine really builds: a full journal is also a limit, and a limit is
        // also invalid.
        assert_eq!(
            Error::new(Kind::Invalid, "x")
                .also(Kind::Limit)
                .also(Kind::JournalFull)
                .code(),
            "journal_full"
        );
        assert_eq!(Error::new(Kind::Invalid, "x").also(Kind::Limit).code(), "limit");
        assert_eq!(
            Error::new(Kind::Exists, "x").also(Kind::CorruptLog).code(),
            "corrupt_log"
        );
    }
}
