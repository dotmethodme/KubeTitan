//! What the app tells the user, and what it offers to do about it.
//!
//! `cluster.rs` reports on one string channel that the log stream shares, so
//! every message is tagged with a prefix. The tag decides where the message is
//! shown and whether it comes with a way out — and the tags live here, next to
//! the reader, so producer and consumer cannot drift apart.
//!
//! Errors do not expire. A confirmation is worth six seconds; a failure is worth
//! reading, and dismissing it is the user's call, not a timer's.

use std::time::{Duration, Instant};

/// How long a message that is not a failure stays up.
const INFO_SECS: u64 = 6;

/// A confirmation ("deleted pod x"), or a failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Info,
    Error,
}

/// The way out of a failure, offered as a button beside it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fix {
    /// Build a fresh client from the current context and restart every watcher:
    /// the answer to a token that expired, a network that went away, or a
    /// watcher that died on a 403.
    Reconnect,
}

impl Fix {
    pub fn label(self) -> &'static str {
        match self {
            Fix::Reconnect => "Reconnect",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub text: String,
    pub kind: Kind,
    /// the recovery on offer, when there is one
    pub fix: Option<Fix>,
    /// when an informational message goes away; `None` means it stays put
    pub until: Option<Instant>,
}

impl Notice {
    /// A confirmation: six seconds, then gone.
    pub fn info(text: String) -> Self {
        Notice {
            text,
            kind: Kind::Info,
            fix: None,
            until: Some(Instant::now() + Duration::from_secs(INFO_SECS)),
        }
    }

    /// A failure the user has to see and dismiss.
    pub fn error(text: String) -> Self {
        Notice {
            text,
            kind: Kind::Error,
            fix: None,
            until: None,
        }
    }

    /// A failure the app knows how to recover from.
    pub fn fixable(text: String, fix: Fix) -> Self {
        Notice {
            text,
            kind: Kind::Error,
            fix: Some(fix),
            until: None,
        }
    }

    /// Has this one had its time? Failures never do.
    pub fn expired(&self) -> bool {
        self.until.is_some_and(|until| Instant::now() >= until)
    }
}

// -- the wire tags --

/// A one-off confirmation.
const STATUS: &str = "LOG_STATUS:";
/// A failure with nothing to offer but the sentence itself.
const ERROR: &str = "LOG_ERR:";
/// A failure the app can act on, so the notice carries a fix.
const FAIL: &str = "LOG_FAIL:";
/// The log stream ended. Not a notice: the log pane shows it in place.
const END: &str = "LOG_END:";

/// Does this line belong to the notice channel rather than the log pane?
pub fn is_notice(line: &str) -> bool {
    line.starts_with(STATUS)
        || line.starts_with(ERROR)
        || line.starts_with(FAIL)
        || line.starts_with(END)
}

/// A confirmation, for `cluster.rs` to tag.
pub fn status(text: impl std::fmt::Display) -> String {
    format!("{STATUS} {text}")
}

/// A failure with no recovery to offer.
pub fn err(text: impl std::fmt::Display) -> String {
    format!("{ERROR} {text}")
}

/// A failure the app can recover from.
pub fn fail(text: impl std::fmt::Display) -> String {
    format!("{FAIL} {text}")
}

/// Read a tagged line back into a notice. Anything untagged, and `LOG_END:`,
/// is not one.
pub fn from_wire(raw: &str) -> Option<Notice> {
    if let Some(rest) = raw.strip_prefix(STATUS) {
        return Some(Notice::info(tidy(rest)));
    }
    if let Some(rest) = raw.strip_prefix(ERROR) {
        return Some(Notice::error(tidy(rest)));
    }
    if let Some(rest) = raw.strip_prefix(FAIL) {
        return Some(Notice::fixable(tidy(rest), Fix::Reconnect));
    }
    None
}

/// Failures are shown in a banner, so a long API error has to read as a
/// sentence: kube wraps the API server's reply in its own JSON, and the part a
/// human needs is the `message` field inside it.
fn tidy(raw: &str) -> String {
    let text = raw.trim();
    if let Some(start) = text.find("message: \"") {
        let rest = &text[start + "message: \"".len()..];
        if let Some(end) = closing_quote(rest) {
            return collapse(&unescape(&rest[..end]));
        }
    }
    collapse(text)
}

/// The index of the quote that ends the string, skipping the escaped ones.
fn closing_quote(text: &str) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        match c {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return Some(i),
            _ => escaped = false,
        }
    }
    None
}

/// `\"` back to `"`, the way the API server's own JSON had it.
fn unescape(text: &str) -> String {
    text.replace("\\\"", "\"")
}

/// One line, however the API server wrapped it.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The tag a line carries, if any: the log pane shows lines that were written
/// into the log stream on purpose, and must not show the tag itself.
pub fn strip(line: &str) -> &str {
    for tag in [STATUS, ERROR, FAIL, END] {
        if let Some(rest) = line.strip_prefix(tag) {
            return rest.trim_start();
        }
    }
    line
}

/// Was this line a failure? The log pane tints those.
pub fn is_failure(line: &str) -> bool {
    line.starts_with(ERROR) || line.starts_with(FAIL)
}

#[cfg(test)]
#[path = "notice_tests.rs"]
mod notice_tests;
