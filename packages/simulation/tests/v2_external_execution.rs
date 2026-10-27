use serde_json::{Value, json};
use sf_domain::{cloud_execution as wire, *};
use sf_simulation::{Engine, EngineInput, ExternalAttemptState, ExternalReceiptStatus};

fn uint(n: u64) -> SafeUint {
    n.try_into().unwrap()
}
fn id(s: &str) -> ObjectId {
    s.to_owned().try_into().unwrap()
}
fn piece(n: u64) -> Value {
    json!({"value":{"coefficient":n,"decimal_places":0},"unit":"piece"})
}

// Reduce an existing real material/acceptance fixture to six original cycles
// with two interchangeable devices. It still uses the production compiler,
// Stock, resource capacity, transfer and acceptance consumers.
fn fixture(stock_count: u64) -> Value {
    let mut v: Value = serde_json::from_str(include_str!("fixtures/multi_sink_goal.json")).unwrap();
    for field in ["halls", "lines", "routes", "products", "boms"] {
        v["project"][field].as_array_mut().unwrap().truncate(1);
    }
    v["project"]["halls"][0]["input_locations"] = json!(["L1-SOURCE"]);
    v["project"]["halls"][0]["output_locations"] = json!(["L1-SINK"]);
    v["project"]["lines"][0]["members"] = json!(["L1-D01", "L1-D02"]);
    v["project"]["lines"][0]["boundary_ports"] = json!([]);
    v["project"]["devices"].as_array_mut().unwrap().truncate(2);
    v["project"]["resources"]
        .as_array_mut()
        .unwrap()
        .truncate(2);
    v["project"]["routes"][0]["steps"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    v["project"]["locations"]
        .as_array_mut()
        .unwrap()
        .retain(|l| matches!(l["id"].as_str(), Some("L1-SOURCE" | "L1-SINK")));
    v["project"]["connections"]
        .as_array_mut()
        .unwrap()
        .truncate(4);
    for c in v["project"]["connections"].as_array_mut().unwrap() {
        let d = &mut c["data"];
        if d["destination"]["kind"] == "port" {
            d["source"]["data"] = json!("L1-SOURCE");
            d["destination_store"] = json!("L1-SOURCE");
        } else {
            d["destination"]["data"] = json!("L1-SINK");
            d["destination_store"] = json!("L1-SINK");
        }
    }
    v["scenario"]["production_tasks"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    v["scenario"]["production_tasks"][0]["planned_quantity"] = piece(6);
    v["scenario"]["acceptance_goal"]["additional_acceptance_scopes"] = json!([]);
    let seed = v["scenario"]["initial_conditions"]["items"][0].clone();
    v["scenario"]["initial_conditions"]["items"] = Value::Array(
        (0..stock_count)
            .map(|n| {
                let mut item = seed.clone();
                item["id"] = json!(format!("blank-{n}"));
                item
            })
            .collect(),
    );
    v
}
fn input(v: Value) -> EngineInput {
    let scenario: ScenarioInput = serde_json::from_value(v["scenario"].clone()).unwrap();
    EngineInput {
        run_id: "external-original-run".to_owned().try_into().unwrap(),
        input_ref: serde_json::from_value(
            json!({"project_id":v["project"]["id"],"project_revision":v["project"]["revision"],
            "scenario_id":scenario.id,"scenario_revision":scenario.revision,
            "frozen_project_file":{"path":"unit/project.json","kind":"project"},
            "frozen_scenario_file":{"path":"unit/scenario.json","kind":"scenario"}}),
        )
        .unwrap(),
        project: serde_json::from_value(v["project"].clone()).unwrap(),
        definitions: serde_json::from_value(v["definitions"].clone()).unwrap(),
        scope: scenario.scope.clone(),
        scenario,
    }
}
fn identity() -> wire::ExecutionIdentity {
    serde_json::from_value(
        json!({"session_id":"session-one","execution_epoch_id":"epoch-one",
        "project_id":"NFR-B60-balanced","host_context_session_id":"host-one"}),
    )
    .unwrap()
}
fn policy() -> wire::ExecutionPolicy {
    serde_json::from_value(
        json!({"clock_mode":"host_monotonic","report_cutoff_ms":10000,
        "stop_new_dispatch_on_cutoff":true,"finish_policy":"drain_admitted_then_finish",
        "target_quantity":piece(6),"source_prediction_window":{"start_ms":0,"end_ms":1200000},
        "observation_max_age_ms":1000}),
    )
    .unwrap()
}
fn binding(device: &str) -> wire::CandidateBinding {
    serde_json::from_value(json!({"binding_id":format!("binding-{device}"),"device_id":device,
        "device_module_ref":{"id":"p201.b60.station","version":"1.0.0"},
        "operation_ref":{"id":"p201.b60.station","version":"1.0.0"},
        "execution_capacity_ref":{"kind":"resource","id":format!("{device}-CAP")},
        "input_ports":[{"role_id":"workpiece","port_id":"input","location_id":"L1-SOURCE","item_type_id":"benchmark-item-v1","unit_id":"piece"}],
        "output_ports":[{"role_id":"workpiece","port_id":"output","location_id":"L1-SINK","item_type_id":"benchmark-item-v1","unit_id":"piece"}],
        "resources":[],"phase_id":"cycle","nominal_work_ms":2000})).unwrap()
}
fn engine(v: Value) -> Engine {
    Engine::new_external(
        input(v),
        identity(),
        policy(),
        vec![binding("L1-D01"), binding("L1-D02")],
    )
    .unwrap()
}
fn attempt(job: ObjectId, device: &str, name: &str, ordinal: u64) -> wire::AttemptRef {
    serde_json::from_value(json!({"logical_job_id":job,"phase_id":"cycle","attempt_ordinal":ordinal,
        "command_id":name,"selected_binding_id":format!("binding-{device}"),"phase_token":format!("token-{name}")})).unwrap()
}
fn prepare(e: &mut Engine, index: usize, device: &str, name: &str, at: u64) -> wire::Command {
    let job = e.external_observation().unwrap().queued_logical_jobs[index].clone();
    e.prepare_external(
        attempt(job, device, name, 1),
        "boot-one".to_owned().try_into().unwrap(),
        uint(at),
    )
    .unwrap()
}
fn publish(e: &mut Engine, c: &wire::Command, at: u64) {
    e.mark_external_published(&c.attempt.command_id, uint(at))
        .unwrap();
}
fn receipt(c: &wire::Command, sequence: u64, kind: wire::DeviceReceiptKind) -> wire::DeviceReceipt {
    wire::DeviceReceipt {
        session_id: c.identity.session_id.clone(),
        execution_epoch_id: c.identity.execution_epoch_id.clone(),
        command_id: c.attempt.command_id.clone(),
        device_id: c.device_id.clone(),
        phase_id: c.attempt.phase_id.clone(),
        phase_token: c.attempt.phase_token.clone(),
        device_boot_id: c.target_device_boot_id.clone(),
        device_sequence: uint(sequence),
        device_timestamp: "device-clock-is-not-business-time"
            .to_owned()
            .try_into()
            .unwrap(),
        receipt: kind,
    }
}
fn started(c: &wire::Command) -> wire::DeviceReceipt {
    receipt(c, 1, wire::DeviceReceiptKind::Started {})
}
fn completed(c: &wire::Command) -> wire::DeviceReceipt {
    receipt(
        c,
        2,
        wire::DeviceReceiptKind::Completed {
            device_reported_duration_ms: Some(uint(999)),
        },
    )
}
fn states(e: &Engine) -> Value {
    serde_json::to_value(e.snapshot()).unwrap()
}

#[test]
fn prepare_reserves_same_stock_and_capacity_without_physical_or_bound_job_commit() {
    let mut e = engine(fixture(1));
    assert_eq!(
        e.external_observation().unwrap().queued_logical_jobs.len(),
        6
    );
    assert!(e.external_observation().unwrap().jobs.is_empty());
    assert!(
        e.prepare_external(
            attempt(id("no-work"), "L1-D01", "before-activation", 1),
            "boot-one".to_owned().try_into().unwrap(),
            uint(0)
        )
        .is_err()
    );
    e.activate_external().unwrap();
    let before = states(&e);
    let c = prepare(&mut e, 0, "L1-D01", "command-one", 0);
    let view = e.external_observation().unwrap();
    assert_eq!(
        serde_json::to_value(&view.items).unwrap(),
        before["final_items"]
    );
    assert!(view.jobs.is_empty());
    assert!(view.events.is_empty());
    assert!(view.resource_claims.is_empty());
    assert_eq!(view.attempts[0].pending_items, vec![(id("blank-0"), 1)]);
    let job = view.queued_logical_jobs[1].clone();
    assert_eq!(
        e.prepare_external(
            attempt(job.clone(), "L1-D01", "busy", 1),
            c.target_device_boot_id.clone(),
            uint(0)
        )
        .unwrap_err()
        .code,
        "external.resources"
    );
    assert_eq!(
        e.prepare_external(
            attempt(job, "L1-D02", "empty", 1),
            c.target_device_boot_id.clone(),
            uint(0)
        )
        .unwrap_err()
        .code,
        "external.material"
    );
    assert_eq!(e.external_observation().unwrap().attempts.len(), 1);
    e.withdraw_external_unpublished(&c.attempt.command_id)
        .unwrap();
    let replacement = e
        .prepare_external(
            attempt(c.attempt.logical_job_id.clone(), "L1-D02", "replacement", 2),
            c.target_device_boot_id.clone(),
            uint(0),
        )
        .unwrap();
    assert_eq!(replacement.device_id, id("L1-D02"));
}

#[test]
fn valid_start_commits_once_and_only_receipt_completes_the_original_production_path() {
    let mut e = engine(fixture(6));
    e.activate_external().unwrap();
    let c = prepare(&mut e, 0, "L1-D02", "command-one", 0);
    publish(&mut e, &c, 0);
    let r = started(&c);
    assert_eq!(
        e.apply_external_receipt(r.clone(), uint(10))
            .unwrap()
            .status,
        ExternalReceiptStatus::Started
    );
    let at_start = e.external_observation().unwrap();
    assert_eq!(at_start.jobs.len(), 1);
    assert_eq!(
        at_start
            .items
            .iter()
            .filter(|i| matches!(i.lifecycle, Lifecycle::InProcess))
            .count(),
        1
    );
    assert!(
        matches!(&at_start.jobs[0].execution_ref,JobExecutionRef::ProjectDevice{device_id,..} if *device_id==id("L1-D02"))
    );
    let allocations = states(&e)["allocations"].clone();
    assert_eq!(allocations.as_array().unwrap().len(), 1);
    assert!(e.apply_external_receipt(r, uint(10)).unwrap().duplicate);
    assert_eq!(states(&e)["allocations"], allocations);
    e.advance_external(uint(3000)).unwrap();
    assert_eq!(e.step(), 0);
    e.run_to_end();
    assert_eq!(e.external_observation().unwrap().elapsed_ms, 3000);
    assert!(
        e.external_observation()
            .unwrap()
            .acceptance_events
            .is_empty()
    );
    let result = e.apply_external_receipt(completed(&c), uint(3100)).unwrap();
    assert_eq!(result.status, ExternalReceiptStatus::Completed);
    assert_eq!(result.device_reported_duration_ms, Some(999));
    let done = e.external_observation().unwrap();
    assert_eq!(done.acceptance_events.len(), 1);
    assert_eq!(done.acceptance_events[0].at_ms.0.value(), 3100);
    assert!(
        matches!(&done.items.iter().find(|i| i.id==id("blank-0")).unwrap().holder,HolderRef::Location(l) if *l==id("L1-SINK"))
    );
    assert_eq!(done.phase_work_intervals[0].interval.start_ms.0.value(), 10);
    assert_eq!(done.phase_work_intervals[0].interval.end_ms.0.value(), 3100);
    assert!(
        e.apply_external_receipt(completed(&c), uint(3100))
            .unwrap()
            .duplicate
    );
    assert_eq!(e.external_observation().unwrap().acceptance_events.len(), 1);
}

#[test]
fn mismatched_identity_and_sequence_conflict_cannot_change_ownership() {
    let mut e = engine(fixture(6));
    e.activate_external().unwrap();
    let c = prepare(&mut e, 0, "L1-D01", "identity-command", 0);
    publish(&mut e, &c, 0);
    let base = serde_json::to_value(started(&c)).unwrap();
    let before = states(&e);
    for key in [
        "session_id",
        "execution_epoch_id",
        "command_id",
        "device_id",
        "device_boot_id",
        "phase_id",
        "phase_token",
    ] {
        let mut bad = base.clone();
        bad[key] = json!("wrong-identity");
        assert!(
            e.apply_external_receipt(serde_json::from_value(bad).unwrap(), uint(5))
                .is_err(),
            "{key}"
        );
        assert_eq!(states(&e), before, "{key}");
    }
    e.apply_external_receipt(started(&c), uint(5)).unwrap();
    let before = states(&e);
    assert_eq!(
        e.apply_external_receipt(
            receipt(
                &c,
                1,
                wire::DeviceReceiptKind::Completed {
                    device_reported_duration_ms: None
                }
            ),
            uint(5)
        )
        .unwrap_err()
        .code,
        "external.sequence"
    );
    assert!(
        e.apply_external_receipt(receipt(&c, 2, wire::DeviceReceiptKind::Started {}), uint(5))
            .is_err()
    );
    assert_eq!(states(&e), before);
}

#[test]
fn early_complete_is_retained_without_synthetic_start_then_reconciled_exactly_once() {
    let mut e = engine(fixture(6));
    e.activate_external().unwrap();
    let c = prepare(&mut e, 0, "L1-D01", "early-command", 0);
    publish(&mut e, &c, 0);
    let out = e.apply_external_receipt(completed(&c), uint(20)).unwrap();
    assert_eq!(out.status, ExternalReceiptStatus::RetainedUnapplied);
    let pending = e.external_observation().unwrap();
    assert!(pending.jobs.is_empty());
    assert!(pending.acceptance_events.is_empty());
    assert_eq!(pending.attempts[0].unapplied_receipts.len(), 1);
    assert!(
        e.apply_external_receipt(completed(&c), uint(20))
            .unwrap()
            .duplicate
    );
    let mut conflict = completed(&c);
    conflict.receipt = wire::DeviceReceiptKind::Completed {
        device_reported_duration_ms: None,
    };
    assert_eq!(
        e.apply_external_receipt(conflict, uint(20))
            .unwrap_err()
            .code,
        "external.sequence"
    );
    assert!(
        e.apply_external_receipt(
            receipt(&c, 3, wire::DeviceReceiptKind::CancelledNotStarted {}),
            uint(20)
        )
        .is_err()
    );
    e.apply_external_receipt(started(&c), uint(21)).unwrap();
    e.apply_external_receipt(completed(&c), uint(22)).unwrap();
    let done = e.external_observation().unwrap();
    assert_eq!(done.acceptance_events.len(), 1);
    assert_eq!(done.jobs.len(), 1);
    assert!(done.attempts[0].unapplied_receipts.is_empty());
}

#[test]
fn published_unknown_cannot_reassign_but_proven_not_started_cancellation_can() {
    let mut e = engine(fixture(6));
    e.activate_external().unwrap();
    let c = prepare(&mut e, 0, "L1-D01", "cancel-command", 0);
    publish(&mut e, &c, 0);
    assert!(
        e.withdraw_external_unpublished(&c.attempt.command_id)
            .is_err()
    );
    let second = attempt(
        c.attempt.logical_job_id.clone(),
        "L1-D02",
        "retry-command",
        2,
    );
    assert_eq!(
        e.prepare_external(second.clone(), c.target_device_boot_id.clone(), uint(0))
            .unwrap_err()
            .code,
        "external.live_attempt"
    );
    e.apply_external_receipt(
        receipt(&c, 1, wire::DeviceReceiptKind::Accepted {}),
        uint(1),
    )
    .unwrap();
    e.apply_external_receipt(
        receipt(&c, 2, wire::DeviceReceiptKind::CancelledNotStarted {}),
        uint(2),
    )
    .unwrap();
    assert!(e.external_observation().unwrap().jobs.is_empty());
    let retry = e
        .prepare_external(second, c.target_device_boot_id.clone(), uint(3))
        .unwrap();
    publish(&mut e, &retry, 3);
    e.apply_external_receipt(started(&retry), uint(4)).unwrap();
    let before = states(&e);
    assert!(
        e.apply_external_receipt(
            receipt(&retry, 2, wire::DeviceReceiptKind::CancelledNotStarted {}),
            uint(4)
        )
        .is_err()
    );
    assert_eq!(states(&e), before);
    assert!(
        e.prepare_external(
            attempt(
                retry.attempt.logical_job_id.clone(),
                "L1-D01",
                "illegal-third",
                3
            ),
            retry.target_device_boot_id.clone(),
            uint(4)
        )
        .is_err()
    );
}

#[test]
fn cutoff_stops_dispatch_and_keeps_an_immutable_prefix_while_admitted_work_drains() {
    let mut e = engine(fixture(6));
    e.activate_external().unwrap();
    let c = prepare(&mut e, 0, "L1-D01", "late-command", 0);
    publish(&mut e, &c, 0);
    let unpublished = prepare(&mut e, 1, "L1-D02", "unpublished", 1);
    e.advance_external(uint(10000)).unwrap();
    let cutoff = e.external_cutoff_observation().unwrap().clone();
    assert_eq!(cutoff.elapsed_ms, 10000);
    assert!(cutoff.dispatch_stopped);
    assert!(!cutoff.drained);
    assert!(cutoff.jobs.is_empty());
    assert!(cutoff.acceptance_events.is_empty());
    assert!(cutoff.attempts.iter().any(|a| a.command.attempt.command_id
        == unpublished.attempt.command_id
        && a.state == ExternalAttemptState::WithdrawnUnpublished));
    let job = e.external_observation().unwrap().queued_logical_jobs[2].clone();
    assert_eq!(
        e.prepare_external(
            attempt(job, "L1-D02", "after-cutoff", 1),
            c.target_device_boot_id.clone(),
            uint(10001)
        )
        .unwrap_err()
        .code,
        "external.cutoff"
    );
    e.apply_external_receipt(started(&c), uint(1200010))
        .unwrap();
    e.apply_external_receipt(completed(&c), uint(1200050))
        .unwrap();
    let live = e.external_observation().unwrap();
    assert_eq!(live.elapsed_ms, 1200050);
    assert!(live.drained);
    assert!(!live.goal_met);
    assert_eq!(live.acceptance_events[0].at_ms.0.value(), 1200050);
    assert_eq!(
        e.external_cutoff_observation().unwrap().elapsed_ms,
        cutoff.elapsed_ms
    );
    assert!(e.external_cutoff_observation().unwrap().jobs.is_empty());
    assert!(
        e.external_cutoff_observation()
            .unwrap()
            .acceptance_events
            .is_empty()
    );
    assert_eq!(
        e.advance_external(uint(1200049)).unwrap_err().code,
        "external.clock"
    );
}

#[test]
fn six_logical_jobs_choose_one_of_two_devices_without_duplicate_outputs() {
    let mut e = engine(fixture(6));
    e.activate_external().unwrap();
    for n in 0..6 {
        let device = if n % 2 == 0 { "L1-D01" } else { "L1-D02" };
        let at = n * 100;
        let c = prepare(&mut e, 0, device, &format!("six-{n}"), at);
        publish(&mut e, &c, at);
        e.apply_external_receipt(started(&c), uint(at + 1)).unwrap();
        e.apply_external_receipt(completed(&c), uint(at + 80))
            .unwrap();
    }
    let done = e.external_observation().unwrap();
    assert_eq!(done.jobs.len(), 6);
    assert_eq!(done.items.len(), 6);
    assert_eq!(done.acceptance_events.len(), 6);
    assert!(done.queued_logical_jobs.is_empty());
    assert!(done.goal_met);
    assert!(done.drained);
    assert_eq!(done.attempts.len(), 6);
    assert_eq!(done.elapsed_ms, 580);
    e.advance_external(uint(10000)).unwrap();
    assert_eq!(e.external_observation().unwrap().elapsed_ms, 580);
    assert!(e.external_cutoff_observation().is_none());
}

#[test]
fn batch_reservations_follow_the_actual_remaining_child_after_partial_take() {
    let mut v = fixture(1);
    v["project"]["item_types"][0]["tracking"] = json!("batch");
    v["definitions"][3]["payload"]["data"]["tracking"] = json!("batch");
    v["scenario"]["initial_conditions"]["items"][0]["quantity"] = piece(6);
    let mut e = engine(v);
    e.activate_external().unwrap();
    let a = prepare(&mut e, 0, "L1-D01", "batch-a", 0);
    publish(&mut e, &a, 0);
    let b = prepare(&mut e, 1, "L1-D02", "batch-b", 0);
    publish(&mut e, &b, 0);
    assert!(
        e.external_observation()
            .unwrap()
            .attempts
            .iter()
            .all(|a| a.pending_items == vec![(id("blank-0"), 1)])
    );
    e.apply_external_receipt(started(&a), uint(1)).unwrap();
    let view = e.external_observation().unwrap();
    let remaining = view
        .items
        .iter()
        .find(|i| {
            matches!(i.lifecycle, Lifecycle::Available) && i.quantity.value.coefficient.value() == 5
        })
        .unwrap();
    assert!(
        view.attempts
            .iter()
            .find(|a| a.command.attempt.command_id == b.attempt.command_id)
            .unwrap()
            .pending_items
            == vec![(remaining.id.clone(), 1)]
    );
    e.apply_external_receipt(started(&b), uint(2)).unwrap();
    e.apply_external_receipt(completed(&a), uint(3)).unwrap();
    e.apply_external_receipt(completed(&b), uint(4)).unwrap();
    let view = e.external_observation().unwrap();
    assert_eq!(
        view.acceptance_events
            .iter()
            .map(|a| a.quantity.value.coefficient.value())
            .sum::<u64>(),
        2
    );
    assert_eq!(
        view.items
            .iter()
            .filter(|i| matches!(i.lifecycle, Lifecycle::Available))
            .map(|i| i.quantity.value.coefficient.value())
            .sum::<u64>(),
        6
    );
}

#[test]
fn completed_command_with_full_sink_retains_original_pending_output_and_claim() {
    let mut v = fixture(6);
    v["project"]["locations"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["id"] == "L1-SINK")
        .unwrap()["capacity"] = piece(1);
    let mut blocker = v["scenario"]["initial_conditions"]["items"][0].clone();
    blocker["id"] = json!("preexisting-sink-item");
    blocker["holder"]["id"] = json!("L1-SINK");
    v["scenario"]["initial_conditions"]["items"]
        .as_array_mut()
        .unwrap()
        .push(blocker);
    let mut e = engine(v);
    e.activate_external().unwrap();
    let c = prepare(&mut e, 0, "L1-D01", "blocked-output", 0);
    publish(&mut e, &c, 0);
    e.apply_external_receipt(started(&c), uint(1)).unwrap();
    assert_eq!(
        e.apply_external_receipt(completed(&c), uint(50))
            .unwrap()
            .status,
        ExternalReceiptStatus::Completed
    );
    let view = e.external_observation().unwrap();
    assert!(view.acceptance_events.is_empty());
    assert!(!view.goal_met);
    assert!(!view.drained);
    assert!(matches!(view.jobs[0].state, JobStatus::WaitingOutput));
    assert!(!view.jobs[0].pending_outputs.is_empty());
    assert!(
        view.items
            .iter()
            .any(|i| matches!(i.holder, HolderRef::Job(_)))
    );
    assert!(!view.jobs[0].job_resources.is_empty());
    let next = view.queued_logical_jobs[0].clone();
    assert_eq!(
        e.prepare_external(
            attempt(next, "L1-D01", "still-busy", 1),
            c.target_device_boot_id.clone(),
            uint(51)
        )
        .unwrap_err()
        .code,
        "external.resources"
    );
    e.advance_external(uint(10000)).unwrap();
    assert!(!e.external_observation().unwrap().drained);
}

#[test]
fn started_failure_preserves_wip_and_resources_without_rollback() {
    let mut e = engine(fixture(6));
    e.activate_external().unwrap();
    let c = prepare(&mut e, 0, "L1-D01", "failed-work", 0);
    publish(&mut e, &c, 0);
    e.apply_external_receipt(started(&c), uint(1)).unwrap();
    let failed = receipt(
        &c,
        2,
        wire::DeviceReceiptKind::Failed {
            code: "fault".to_owned().try_into().unwrap(),
            message: "stopped with work in fixture"
                .to_owned()
                .try_into()
                .unwrap(),
        },
    );
    assert_eq!(
        e.apply_external_receipt(failed.clone(), uint(10))
            .unwrap()
            .status,
        ExternalReceiptStatus::Failed
    );
    assert!(
        e.apply_external_receipt(failed, uint(10))
            .unwrap()
            .duplicate
    );
    e.advance_external(uint(10000)).unwrap();
    let view = e.external_observation().unwrap();
    assert!(!view.drained);
    assert!(view.acceptance_events.is_empty());
    assert!(
        view.items
            .iter()
            .any(|i| matches!(i.lifecycle, Lifecycle::InProcess)
                && matches!(i.holder, HolderRef::Job(_)))
    );
    assert!(!view.jobs[0].job_resources.is_empty());
    assert!(matches!(view.jobs[0].state, JobStatus::Paused));
    assert_eq!(view.phase_work_intervals[0].interval.end_ms.0.value(), 10);
    assert!(
        e.apply_external_receipt(completed(&c), uint(10001))
            .is_err()
    );
}

#[test]
fn completion_observed_at_cutoff_is_part_of_its_frozen_prefix() {
    let mut e = engine(fixture(6));
    e.activate_external().unwrap();
    let c = prepare(&mut e, 0, "L1-D01", "at-cutoff", 0);
    publish(&mut e, &c, 0);
    e.apply_external_receipt(started(&c), uint(1)).unwrap();
    e.apply_external_receipt(completed(&c), uint(10000))
        .unwrap();
    let cutoff = e.external_cutoff_observation().unwrap();
    assert_eq!(cutoff.acceptance_events.len(), 1);
    assert_eq!(cutoff.acceptance_events[0].at_ms.0.value(), 10000);
    assert!(cutoff.drained);
}

#[test]
fn stop_dispatch_withdraws_only_unpublished_work_and_does_not_cancel_started_work() {
    let mut e = engine(fixture(6));
    e.activate_external().unwrap();
    let a = prepare(&mut e, 0, "L1-D01", "safe-unpublished", 0);
    let b = prepare(&mut e, 1, "L1-D02", "active-after-stop", 0);
    publish(&mut e, &b, 0);
    e.apply_external_receipt(started(&b), uint(1)).unwrap();
    e.cancel();
    let view = e.external_observation().unwrap();
    assert!(view.dispatch_stopped);
    assert!(!view.drained);
    assert_eq!(
        view.attempts
            .iter()
            .find(|x| x.command.attempt.command_id == a.attempt.command_id)
            .unwrap()
            .state,
        ExternalAttemptState::WithdrawnUnpublished
    );
    e.apply_external_receipt(completed(&b), uint(2)).unwrap();
    assert!(e.external_observation().unwrap().drained);
}

#[test]
fn candidate_mappings_are_checked_against_the_original_compiler() {
    let base = serde_json::to_value(binding("L1-D01")).unwrap();
    for (pointer, bad) in [
        ("/device_module_ref/version", json!("9.0.0")),
        ("/input_ports/0/unit_id", json!("kg")),
        ("/input_ports/0/port_id", json!("absent")),
        ("/output_ports/0/location_id", json!("L1-SOURCE")),
        ("/execution_capacity_ref/id", json!("L1-D02-CAP")),
        ("/nominal_work_ms", json!(1)),
        ("/phase_id", json!("other")),
    ] {
        let mut candidate = base.clone();
        *candidate.pointer_mut(pointer).unwrap() = bad;
        assert!(
            Engine::new_external(
                input(fixture(6)),
                identity(),
                policy(),
                vec![serde_json::from_value(candidate).unwrap()]
            )
            .is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn original_local_mode_still_uses_nominal_completion_and_same_stock() {
    let mut e = Engine::new(input(fixture(6))).unwrap();
    e.run_to_end();
    let run = e.snapshot();
    assert!(run.whole_window_computed);
    assert_eq!(run.jobs.len(), 6);
    assert_eq!(run.acceptance_events.len(), 6);
    assert_eq!(run.acceptance_events[0].at_ms.0.value(), 2000);
    assert_eq!(run.acceptance_events[5].at_ms.0.value(), 12000);
}

#[test]
fn unsupported_multiphase_route_is_explicitly_rejected() {
    let mut v = fixture(6);
    let mut extra =
        v["definitions"][0]["payload"]["data"]["behavior"]["inline"]["phases"][0].clone();
    extra["id"] = json!("second-stage");
    v["definitions"][0]["payload"]["data"]["behavior"]["inline"]["phases"]
        .as_array_mut()
        .unwrap()
        .push(extra);
    let result = Engine::new_external(input(v), identity(), policy(), vec![binding("L1-D01")]);
    assert!(matches!(result,Err(e) if e.code=="external.unsupported"));
}

#[test]
fn cancellation_rejected_as_started_requires_original_start_reconciliation() {
    let mut e = engine(fixture(6));
    e.activate_external().unwrap();
    let c = prepare(&mut e, 0, "L1-D01", "reconcile-start", 0);
    publish(&mut e, &c, 0);
    let rejected = receipt(&c, 2, wire::DeviceReceiptKind::CancelRejectedStarted {});
    assert_eq!(
        e.apply_external_receipt(rejected.clone(), uint(5))
            .unwrap()
            .status,
        ExternalReceiptStatus::RetainedUnapplied
    );
    e.apply_external_receipt(
        receipt(&c, 0, wire::DeviceReceiptKind::Accepted {}),
        uint(6),
    )
    .unwrap();
    let pending = e.external_observation().unwrap();
    assert!(pending.jobs.is_empty());
    assert_eq!(
        pending.attempts[0].state,
        ExternalAttemptState::ReconciliationRequired
    );
    e.apply_external_receipt(started(&c), uint(7)).unwrap();
    let allocations = states(&e)["allocations"].clone();
    assert_eq!(
        e.apply_external_receipt(rejected, uint(8)).unwrap().status,
        ExternalReceiptStatus::Started
    );
    assert_eq!(states(&e)["allocations"], allocations);
    e.apply_external_receipt(
        receipt(
            &c,
            3,
            wire::DeviceReceiptKind::Completed {
                device_reported_duration_ms: None,
            },
        ),
        uint(9),
    )
    .unwrap();
    let done = e.external_observation().unwrap();
    assert_eq!(done.acceptance_events.len(), 1);
    assert!(done.attempts[0].unapplied_receipts.is_empty());
}
