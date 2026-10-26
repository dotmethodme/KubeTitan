//! The invariant a forward has to keep: what the user asked for is what gets
//! saved, however the target resolves.
//!
//! A service's `port` and its `targetPort` are different numbers on purpose
//! (`argocd-server` is 80 to the cluster, 8080 inside the pod). The tunnel has
//! to be opened to the pod's port, and the saved forward has to record the
//! service's — writing the resolved one over it is what put 8080 in a saved
//! forward that asked for 80.

use super::*;

fn forward(target: Target, remote_port: u16) -> Forward {
    Forward {
        id: "pf1".to_string(),
        namespace: "argocd".to_string(),
        target,
        name: "argocd-server".to_string(),
        pod: "argocd-server".to_string(),
        local_port: 8003,
        remote_port,
        target_port: remote_port,
        status: Status::Active,
        error: None,
    }
}

#[test]
fn resolving_a_service_keeps_the_port_that_was_asked_for() {
    let mut forward = forward(Target::Service, 80);
    forward.resolved("argocd-server-6f9c8d-abc12".to_string(), 8080);
    assert_eq!(forward.remote_port, 80);
    assert_eq!(forward.target_port, 8080);
    assert_eq!(forward.pod, "argocd-server-6f9c8d-abc12");
    // and it is the asked-for port that is written down
    assert_eq!(forward.saved().remote_port, 80);
}

#[test]
fn a_pod_forward_resolves_to_the_same_port() {
    let mut forward = forward(Target::Pod, 6379);
    forward.resolved("snakdansk-redis-0".to_string(), 6379);
    assert_eq!(forward.remote_port, 6379);
    assert_eq!(forward.target_port, 6379);
    assert_eq!(forward.saved().remote_port, 6379);
}

#[test]
fn a_saved_forward_comes_back_asking_for_the_same_port() {
    let saved = Saved {
        id: "pf4".to_string(),
        namespace: "argocd".to_string(),
        target: Target::Service,
        name: "argocd-server".to_string(),
        local_port: 8003,
        remote_port: 80,
    };
    let back = saved.into_forward();
    assert_eq!(back.remote_port, 80);
    // nothing has resolved yet, so the port to open is the one asked for
    assert_eq!(back.target_port, 80);
    assert_eq!(back.status, Status::Stopped);
}
