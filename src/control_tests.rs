//! Tests for the agent control channel's protocol. An external agent depends on
//! this surface, so it is pinned here: the JSON it sends, and the replies it can
//! rely on. `apply` itself needs a live `App`, which the headless harness drives
//! over the real channel instead.

use super::*;

#[test]
fn commands_parse_from_the_documented_json() {
    let nav: Command = serde_json::from_str(r#"{"cmd":"nav","view":"deployments"}"#).unwrap();
    assert!(matches!(nav, Command::Nav { view } if view == "deployments"));

    let ns: Command = serde_json::from_str(r#"{"cmd":"namespace","name":"dev"}"#).unwrap();
    assert!(matches!(ns, Command::Namespace { name } if name == "dev"));

    let select: Command = serde_json::from_str(r#"{"cmd":"select","key":"dev/x"}"#).unwrap();
    assert!(matches!(select, Command::Select { key } if key == "dev/x"));

    // an omitted local port means Auto, as an empty field does in the dialog
    let forward: Command = serde_json::from_str(
        r#"{"cmd":"forward","namespace":"dev","target":"pod","name":"x","remote_port":8080}"#,
    )
    .unwrap();
    assert!(matches!(
        forward,
        Command::Forward {
            local_port,
            remote_port,
            ..
        } if local_port == 0 && remote_port == 8080
    ));

    assert!(matches!(
        serde_json::from_str::<Command>(r#"{"cmd":"stop_forward","id":"pf1"}"#).unwrap(),
        Command::StopForward { id } if id == "pf1"
    ));

    assert!(matches!(
        serde_json::from_str::<Command>(r#"{"cmd":"quit"}"#).unwrap(),
        Command::Quit
    ));
}

#[test]
fn an_unknown_command_is_a_clean_error() {
    // a typo must not take the app down with it
    let err = serde_json::from_str::<Command>(r#"{"cmd":"nope"}"#).unwrap_err();
    assert!(err.to_string().contains("nope"));
}

#[test]
fn actions_are_named_like_the_context_menu() {
    assert!(matches!(parse_action("scale"), Some(Action::Scale)));
    assert!(matches!(parse_action("restart"), Some(Action::Restart)));
    assert!(matches!(parse_action("forward"), Some(Action::Forward)));
    assert!(matches!(parse_action("delete"), Some(Action::Delete)));
    // server-side apply is its own flow in the drawer, not a row action
    assert!(parse_action("apply").is_none());
}

#[test]
fn replies_carry_only_what_is_set() {
    let json = serde_json::to_string(&failed("boom".to_string())).unwrap();
    assert_eq!(json, r#"{"ok":false,"error":"boom"}"#);
}
