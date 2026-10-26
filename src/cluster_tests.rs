//! Tests for the cluster layer's pure helpers. The watcher, the pollers and
//! log streaming touch the apiserver, so they are exercised against the live
//! cluster by the headless UI harness instead.

use super::*;
use serde_json::json;

fn pod_with_containers(containers: serde_json::Value) -> DynamicObject {
    serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": "blog-0", "namespace": "blog"},
        "spec": {"containers": containers},
    }))
    .expect("fixture is a valid pod")
}

#[test]
fn pod_containers_keeps_spec_order() {
    // the first container is what the drawer streams and charts, so order matters
    let pod = pod_with_containers(json!([
        {"name": "hermes"},
        {"name": "sidecar"},
    ]));
    assert_eq!(pod_containers(&pod), vec!["hermes", "sidecar"]);
}

#[test]
fn pod_containers_is_empty_when_the_spec_is_incomplete() {
    // a pending pod may have no containers yet: no panic, no bogus default
    let pod: DynamicObject = serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": "pending"},
        "spec": {},
    }))
    .expect("fixture parses");
    assert!(pod_containers(&pod).is_empty());
}

#[test]
fn init_containers_are_listed_before_the_app_containers() {
    // storywise's pod: five app containers plus an init container the drawer
    // used to hide entirely. Init first, because that is the order they run in.
    let pod: DynamicObject = serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": "storywise-0", "namespace": "postgres-operator"},
        "spec": {
            "initContainers": [{"name": "replication-cert-copy"}],
            "containers": [
                {"name": "database"},
                {"name": "pgbackrest"},
                {"name": "exporter"},
            ],
        },
    }))
    .expect("fixture parses");

    assert_eq!(pod_init_containers(&pod), vec!["replication-cert-copy"]);
    assert_eq!(
        pod_containers(&pod),
        vec!["database", "pgbackrest", "exporter"],
        "the app containers keep spec order, and the default log target stays the first of them"
    );
    assert_eq!(
        pod_all_containers(&pod),
        vec![
            "replication-cert-copy",
            "database",
            "pgbackrest",
            "exporter"
        ]
    );
}

#[test]
fn ephemeral_containers_come_last() {
    // `kubectl debug` copies live alongside the app containers, not among them
    let pod: DynamicObject = serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": "debugged"},
        "spec": {
            "containers": [{"name": "app"}],
            "ephemeralContainers": [{"name": "debugger-abc"}],
        },
    }))
    .expect("fixture parses");

    assert_eq!(pod_containers(&pod), vec!["app"]);
    assert_eq!(pod_ephemeral_containers(&pod), vec!["debugger-abc"]);
    assert_eq!(pod_all_containers(&pod), vec!["app", "debugger-abc"]);
}

#[test]
fn managed_fields_are_stripped_from_an_object() {
    // managedFields is the apiserver's bookkeeping: noise in the YAML tab (the
    // Wails backend strips it too) and a third of every stored object's payload
    let mut value = json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "name": "blog-0",
            "managedFields": [{"manager": "kubectl"}],
            "labels": {"app": "blog"},
        },
    });
    strip_managed_fields(&mut value);
    assert!(value.pointer("/metadata/managedFields").is_none());
    // everything else survives
    assert_eq!(
        value.pointer("/metadata/name").and_then(|v| v.as_str()),
        Some("blog-0")
    );
    assert_eq!(
        value
            .pointer("/metadata/labels/app")
            .and_then(|v| v.as_str()),
        Some("blog")
    );
}

#[test]
fn stripping_managed_fields_survives_objects_without_metadata() {
    let mut value = json!({"apiVersion": "v1", "kind": "List"});
    strip_managed_fields(&mut value);
    assert_eq!(value["kind"], "List");
}

#[test]
fn app_health_matches_the_vue_grid() {
    let app = |ready, desired| AppRow {
        name: "blog".into(),
        kind: "deployment",
        ready,
        desired,
    };
    assert_eq!(app(3, 3).status(), AppStatus::Healthy);
    assert_eq!(app(1, 3).status(), AppStatus::Degraded);
    assert_eq!(app(0, 3).status(), AppStatus::Unavailable);
    // scaled to zero is not "healthy": the Vue grid requires desired > 0
    assert_eq!(app(0, 0).status(), AppStatus::Unavailable);
}

#[test]
fn an_app_card_opens_its_own_kind_view() {
    let card = |kind| AppRow {
        name: "x".into(),
        kind,
        ready: 1,
        desired: 1,
    };
    assert_eq!(card("deployment").view_id(), "deployments");
    assert_eq!(card("statefulset").view_id(), "statefulsets");
}

fn data_pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[test]
fn a_secret_reads_as_its_values() {
    let object: DynamicObject = serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": { "name": "example" },
        "data": { "password": "aHVudGVyMg==", "user": "YWRtaW4=" },
    }))
    .unwrap();
    assert_eq!(
        data_entries(&object, true),
        data_pairs(&[("password", "hunter2"), ("user", "admin")])
    );
}

#[test]
fn a_value_that_is_not_base64_is_shown_as_it_stands() {
    let object: DynamicObject = serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": { "name": "example" },
        "data": { "odd": "not base64!" },
    }))
    .unwrap();
    assert_eq!(
        data_entries(&object, true),
        data_pairs(&[("odd", "not base64!")])
    );
}

#[test]
fn a_configmap_is_already_text() {
    let object: DynamicObject = serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": { "name": "example" },
        "data": { "greeting": "hello" },
    }))
    .unwrap();
    assert_eq!(
        data_entries(&object, false),
        data_pairs(&[("greeting", "hello")])
    );
}

#[test]
fn only_the_changed_keys_are_patched() {
    let original = data_pairs(&[("a", "1"), ("b", "2")]);
    let edited = data_pairs(&[("a", "1"), ("b", "3")]);
    let changed = changed_entries(&original, &edited);
    assert_eq!(changed, data_pairs(&[("b", "3")]));
    assert_eq!(data_patch(&changed, false), json!({ "data": { "b": "3" } }));
    // a secret's values go back base64-encoded
    assert_eq!(
        data_patch(&changed, true),
        json!({ "data": { "b": "Mw==" } })
    );
}
