//! Column extractors, tested against the object shapes real clusters carry.
//!
//! An extractor is a pure function of an object and the row context, so its bugs
//! are the kind a screenshot shows only if the cluster happens to hold that
//! shape. `IntOrString` fields are the case in point: reading one as a number
//! turns "25%" into 0, and a test written against a whole-number object would
//! never have noticed.

use super::*;

fn object(json: serde_json::Value) -> DynamicObject {
    serde_json::from_value(json).expect("a well-formed object")
}

fn cells(
    extractor: fn(&DynamicObject, &RowCtx) -> Vec<Cell>,
    json: serde_json::Value,
) -> Vec<Cell> {
    let metrics = MetricsIndex::default();
    let ctx = RowCtx { metrics: &metrics };
    extractor(&object(json), &ctx)
}

fn pdb(min: serde_json::Value, max: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "apiVersion": "policy/v1",
        "kind": "PodDisruptionBudget",
        "metadata": { "name": "blog", "namespace": "default" },
        "spec": { "minAvailable": min, "maxUnavailable": max },
        "status": { "currentHealthy": 2, "desiredHealthy": 3, "disruptionsAllowed": 1 }
    })
}

#[test]
fn a_percentage_budget_is_shown_not_dropped() {
    // "max 25%" is what most real budgets say, and it is a string in the API
    let rendered = cells(
        cells_pdbs,
        pdb(serde_json::Value::Null, serde_json::json!("25%")),
    );
    assert_eq!(rendered[2].text, "max 25%");
}

#[test]
fn a_whole_number_budget_is_shown_too() {
    let rendered = cells(
        cells_pdbs,
        pdb(serde_json::json!(2), serde_json::Value::Null),
    );
    assert_eq!(rendered[2].text, "min 2");
}

#[test]
fn a_budget_that_says_neither_renders_empty() {
    let rendered = cells(
        cells_pdbs,
        pdb(serde_json::Value::Null, serde_json::Value::Null),
    );
    assert_eq!(rendered[2].text, "");
}

#[test]
fn a_budget_is_healthy_when_it_can_afford_the_disruption() {
    // 2 healthy of 3 desired, so the dot warns — and the same rule draws a
    // workload's dot, which is why one helper serves both
    let rendered = cells(
        cells_pdbs,
        pdb(serde_json::Value::Null, serde_json::json!(1)),
    );
    assert_eq!(rendered[0].dot, Some(theme::WARN));

    let mut healthy = pdb(serde_json::Value::Null, serde_json::json!(1));
    healthy["status"]["currentHealthy"] = serde_json::json!(3);
    assert_eq!(
        cells(cells_pdbs, healthy)[0].dot,
        Some(theme::OK),
        "all desired healthy"
    );

    let mut none = pdb(serde_json::Value::Null, serde_json::json!(1));
    none["status"]["currentHealthy"] = serde_json::json!(0);
    assert_eq!(cells(cells_pdbs, none)[0].dot, Some(theme::BAD));
}

#[test]
fn a_job_reports_its_state_in_the_dot() {
    let job = |succeeded: i64, failed: i64| {
        serde_json::json!({
            "apiVersion": "batch/v1",
            "kind": "Job",
            "metadata": { "name": "backup", "namespace": "default" },
            "spec": { "completions": 1 },
            "status": { "succeeded": succeeded, "failed": failed }
        })
    };
    assert_eq!(cells(cells_jobs, job(1, 0))[0].dot, Some(theme::OK));
    assert_eq!(cells(cells_jobs, job(0, 2))[0].dot, Some(theme::BAD));
    assert_eq!(cells(cells_jobs, job(0, 0))[0].dot, Some(theme::WARN));
}
