//! Tests for the Prometheus layer's pure parts: the query builders, the
//! PromQL/query-string escaping, and the response parser. Everything that
//! touches the network is covered by the headless UI harness instead.

use super::*;

fn body(json: &str) -> serde_json::Value {
    serde_json::from_str(json).expect("test fixture is valid JSON")
}

#[test]
fn parses_a_matrix_response() {
    let series = parse_matrix(&body(
        r#"{"status":"success","data":{"resultType":"matrix","result":[
             {"metric":{"pod":"blog-0"},"values":[[1700000000,"1.5"],[1700000060,"1.25"]]}]}}"#,
    ))
    .expect("a success response parses");
    assert_eq!(series.points, vec![(1700000000, 1.5), (1700000060, 1.25)]);
    assert_eq!(series.latest(), Some(1.25));
}

#[test]
fn an_empty_result_is_no_data_not_an_error() {
    // a query that matches nothing is what a quiet namespace looks like
    let series = parse_matrix(&body(
        r#"{"status":"success","data":{"resultType":"matrix","result":[]}}"#,
    ))
    .expect("an empty result is not an error");
    assert!(series.points.is_empty());
    assert_eq!(series.latest(), None);
}

#[test]
fn a_prometheus_error_is_surfaced() {
    let error = parse_matrix(&body(
        r#"{"status":"error","errorType":"bad_data","error":"invalid parameter \"query\""}"#,
    ))
    .expect_err("an error status is reported");
    assert!(error.contains("invalid parameter"), "got {error}");
}

#[test]
fn skips_samples_that_are_not_finite_numbers() {
    // Prometheus emits "NaN"/"Inf" as strings for some series; a chart must not
    // plot them, and the surviving timestamps must stay in order
    let series = parse_matrix(&body(
        r#"{"status":"success","data":{"resultType":"matrix","result":[
             {"metric":{},"values":[[1,"NaN"],[2,"Inf"],[3,"0.5"],[4,"bogus"],[5,"2"]]}]}}"#,
    ))
    .expect("parses");
    assert_eq!(series.points, vec![(3, 0.5), (5, 2.0)]);
}

#[test]
fn pod_query_matches_the_wails_backend() {
    let [cpu, memory, disk] = pod_queries("blog", "blog-0");
    let selector = r#"namespace="blog",pod="blog-0",container!="",container!="POD""#;
    assert_eq!(
        cpu,
        format!("sum(rate(container_cpu_usage_seconds_total{{{selector}}}[5m]))")
    );
    assert_eq!(
        memory,
        format!("sum(container_memory_working_set_bytes{{{selector}}})")
    );
    assert_eq!(disk, format!("sum(container_fs_usage_bytes{{{selector}}})"));
}

#[test]
fn workload_query_matches_pods_by_quoted_regex() {
    let pods = vec!["blog-0".to_string(), "blog-1".to_string()];
    let [cpu, _, _] = workload_queries("blog", &pods);
    assert!(
        cpu.contains(r#"pod=~"blog-0|blog-1""#),
        "unexpected query: {cpu}"
    );
}

#[test]
fn escaping_quotes_regex_metacharacters() {
    // the dot in a pod name is a regex metacharacter and must be quoted;
    // a dash is not, and Go's regexp.QuoteMeta leaves it alone
    assert_eq!(escape_regex("blog.api-0"), r"blog\.api-0");
    assert_eq!(escape_regex("plainpod"), "plainpod");
    assert_eq!(escape_regex("a+b*c?d"), r"a\+b\*c\?d");
    assert_eq!(escape_regex("[x]{y}(z)"), r"\[x\]\{y\}\(z\)");
}

#[test]
fn encoding_escapes_everything_a_query_string_may_not_carry() {
    // the query travels in a query string, so quotes, braces and spaces encode
    assert_eq!(percent_encode(r#"a="b" {c}"#), "a%3D%22b%22%20%7Bc%7D");
    assert_eq!(percent_encode("A-z0_9.~"), "A-z0_9.~");
    assert_eq!(percent_encode("[5m]"), "%5B5m%5D");
}

#[test]
fn recognises_prometheus_services() {
    let empty = std::collections::BTreeMap::new();
    assert!(looks_like_prometheus("prometheus", "monitoring", &empty));
    assert!(looks_like_prometheus(
        "kube-prometheus-stack-prometheus",
        "observability",
        &empty
    ));
    let labelled = std::collections::BTreeMap::from([(
        "app.kubernetes.io/name".to_string(),
        "prometheus".to_string(),
    )]);
    assert!(looks_like_prometheus("prom-0", "monitoring", &labelled));
    // VictoriaMetrics and friends answer the same API
    assert!(looks_like_prometheus(
        "victoria-metrics",
        "monitoring",
        &empty
    ));

    // a name match alone is not enough to be a *query target*: the operator and
    // its friends are recognised, then filtered out of discovery
    assert!(looks_like_prometheus(
        "prometheus-operator",
        "monitoring",
        &empty
    ));
    assert!(has_excluded_token("prometheus-operator"));
    assert!(has_excluded_token("prometheus-node-exporter"));
    assert!(has_excluded_token("kube-state-metrics"));
    assert!(!has_excluded_token("kube-prometheus-stack-prometheus"));
    assert!(!looks_like_prometheus("my-service", "default", &empty));
}

#[test]
fn a_real_service_ranks_above_a_sidecar() {
    // :9090 is the Prometheus HTTP port; a same-named service on another port
    // must not win
    let native = score("prometheus", "monitoring", 9090);
    let odd = score("prometheus", "monitoring", 9999);
    assert!(native > odd, "{native} should beat {odd}");
}
