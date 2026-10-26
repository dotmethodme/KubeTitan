//! What the forward dialog offers as a default port.
//!
//! This is the one the user gets when they press Start without touching the
//! field, so it has to be the port they would type: a service's `port`, never
//! its `targetPort` — those differ on purpose (`argocd-server` is 80 → 8080),
//! and forwarding to the target port means going to a pod directly instead of
//! through the service.

use super::*;

fn object(json: serde_json::Value) -> kube::core::DynamicObject {
    serde_json::from_value(json).expect("a well-formed object")
}

#[test]
fn a_service_defaults_to_its_own_port_not_the_target_port() {
    // the shape `argocd-server` really has: http 80 -> 8080, https 443 -> 8080
    let service = object(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": { "name": "argocd-server", "namespace": "argocd" },
        "spec": {
            "ports": [
                { "name": "http", "port": 80, "targetPort": 8080, "protocol": "TCP" },
                { "name": "https", "port": 443, "targetPort": 8080, "protocol": "TCP" }
            ]
        }
    }));
    assert_eq!(default_port(&service, "services"), "80");
}

#[test]
fn a_pod_defaults_to_its_first_container_port() {
    let pod = object(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": "blog-8dc9b7b55-n67wg", "namespace": "default" },
        "spec": {
            "containers": [
                { "name": "blog", "image": "nginx", "ports": [{ "containerPort": 80 }] }
            ]
        }
    }));
    assert_eq!(default_port(&pod, "pods"), "80");
}

#[test]
fn a_resource_with_nothing_to_go_on_offers_nothing() {
    let deployment = object(serde_json::json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": { "name": "blog", "namespace": "default" },
        "spec": { "replicas": 1 }
    }));
    assert_eq!(default_port(&deployment, "deployments"), "");
}

#[test]
fn a_service_hint_names_both_numbers() {
    // the line that stops 8080 being typed where 80 belongs
    let service = object(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": { "name": "argocd-server", "namespace": "argocd" },
        "spec": {
            "ports": [
                { "name": "http", "port": 80, "targetPort": 8080, "protocol": "TCP" },
                { "name": "https", "port": 443, "targetPort": 8080, "protocol": "TCP" }
            ]
        }
    }));
    assert_eq!(
        port_hint(&service, "services"),
        "80 (\u{2192} 8080), 443 (\u{2192} 8080)"
    );
}

#[test]
fn a_service_hint_stays_quiet_when_the_numbers_agree() {
    let service = object(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": { "name": "kubetitan", "namespace": "default" },
        "spec": { "ports": [{ "port": 80, "targetPort": 80 }] }
    }));
    assert_eq!(port_hint(&service, "services"), "80");
}

#[test]
fn a_pod_hint_lists_its_container_ports() {
    let pod = object(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": "blog-8dc9b7b55-n67wg", "namespace": "default" },
        "spec": {
            "containers": [
                { "name": "blog", "image": "nginx", "ports": [{ "containerPort": 80 }] }
            ]
        }
    }));
    assert_eq!(port_hint(&pod, "pods"), "80");
}
