//! Tests for the notice channel: the tags `cluster.rs` writes, and what the app
//! makes of them.

use super::*;

#[test]
fn a_confirmation_is_tagged_and_times_out() {
    let notice = from_wire(&status("deleted pod blog-1")).expect("a notice");
    assert_eq!(notice.text, "deleted pod blog-1");
    assert_eq!(notice.kind, Kind::Info);
    assert_eq!(notice.fix, None);
    assert!(
        notice.until.is_some(),
        "a confirmation goes away on its own"
    );
    assert!(!notice.expired(), "but not the moment it appears");
}

#[test]
fn a_failure_never_times_out_and_offers_a_way_out() {
    let notice = from_wire(&fail("pods watcher: connection reset")).expect("a notice");
    assert_eq!(notice.kind, Kind::Error);
    assert_eq!(notice.fix, Some(Fix::Reconnect));
    assert!(
        !notice.expired(),
        "an error waits for the user, however long"
    );
    assert_eq!(Fix::Reconnect.label(), "Reconnect");
}

#[test]
fn a_plain_error_offers_nothing() {
    let notice = from_wire(&err("delete blog-1: forbidden")).expect("a notice");
    assert_eq!(notice.kind, Kind::Error);
    assert_eq!(notice.fix, None);
}

#[test]
fn the_api_servers_json_becomes_a_sentence() {
    // what kube prints when the API server says no: the sentence a human needs
    // is inside `message`, with its quotes escaped
    let raw = err(concat!(
        r#"ApiError: ErrorResponse { status: "Failure", message: "#,
        r#""pods \"blog-1\" is forbidden: User \"system:serviceaccount:default:hermes\" cannot delete resource""#,
        r#", reason: "Forbidden", code: 403 }"#,
    ));
    let notice = from_wire(&raw).expect("a notice");
    assert_eq!(
        notice.text,
        "pods \"blog-1\" is forbidden: User \"system:serviceaccount:default:hermes\" cannot delete resource"
    );
}

#[test]
fn the_log_stream_end_is_not_a_notice() {
    // the log pane shows it in place; a banner would be wrong
    assert!(from_wire("LOG_END: stream closed").is_none());
    assert!(from_wire("12:00:00 INFO serving on :8080").is_none());
}

#[test]
fn every_tag_the_app_knows_is_classified() {
    // so a new tag cannot be added without deciding where its line goes
    assert!(is_notice(&status("scaled blog to 3")));
    assert!(is_notice(&err("boom")));
    assert!(is_notice(&fail("boom")));
    assert!(is_notice("LOG_END: done"));
    assert!(!is_notice("12:00:00 INFO serving on :8080"));
}

#[test]
fn the_log_pane_shows_the_line_without_its_tag() {
    // a status line the app writes into the log stream on purpose
    assert_eq!(
        strip(&status("no pods match this selector yet")),
        "no pods match this selector yet"
    );
    // and a container's own line, tag-free, comes through untouched
    let container = "12:00:00 INFO serving on :8080";
    assert_eq!(strip(container), container);
    assert!(!is_failure(container));
    assert!(is_failure(&err("boom")));
    assert!(is_failure(&fail("boom")));
    assert!(!is_failure(&status("scaled blog to 3")));
}
