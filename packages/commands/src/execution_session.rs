//! Host-owned external execution. This owns the original Engine, never a second
//! stock. Native supplies authenticated messages and the one monotonic clock.
use crate::execution_api::*;
use crate::execution_projection::{
    AttemptMetadata, ExecutionProjector, HostProjectionLedger, HostReceiptEvidence, ProjectionInput,
};
use crate::{ApiError, ErrorCode};
use serde_json::Value;
use sf_domain::{cloud_execution::*, *};
use sf_simulation::{
    Engine, EngineInput, ExternalAttemptState, ExternalObservation, ExternalReceiptStatus,
};
use std::{collections::BTreeMap, time::Instant};

type Result<T> = std::result::Result<T, ApiError>;
fn uint(n: u64) -> SafeUint {
    SafeUint::try_from(n).expect("checked Host execution counter")
}
fn object(value: String) -> ObjectId {
    ObjectId::try_from(value).expect("Host-generated identity")
}
fn fault(code: ErrorCode, message: impl Into<String>) -> ApiError {
    ApiError {
        code,
        message: message.into(),
        object_refs: vec![],
        field_errors: vec![],
        expected_revision: None,
        actual_revision: None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    fn id(s: &str) -> ObjectId {
        object(s.to_owned())
    }
    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }
    fn stamp() -> CloudTimestamp {
        "2026-10-27T00:00:00.123456Z".to_owned().try_into().unwrap()
    }
    fn session_with(cutoff: u64, arrival: bool) -> ExecutionSession {
        // Same original compiler/Stock/acceptance fixture as the Engine batch,
        // with six units and real model parameters fixed to 10/15 seconds.
        let mut v: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/cloud_execution/six_units.json"
        ))
        .unwrap();
        if arrival {
            let item = v["scenario"]["initial_conditions"]["items"]
                .as_array_mut()
                .unwrap()
                .pop()
                .unwrap();
            v["scenario"]["arrivals"] = json!([{"id":"arriving-original-sixth-unit","at_ms":5,"destination":"L1-SOURCE","items":[item],"release_policy":"unchecked_allowed"}]);
        }
        let scenario: ScenarioInput = serde_json::from_value(v["scenario"].clone()).unwrap();
        let input_ref:RunInputRef=serde_json::from_value(json!({"project_id":v["project"]["id"],"project_revision":v["project"]["revision"],
            "scenario_id":scenario.id,"scenario_revision":scenario.revision,"frozen_project_file":{"path":"execution/project.json","kind":"project"},
            "frozen_scenario_file":{"path":"execution/scenario.json","kind":"scenario"}})).unwrap();
        let input = EngineInput {
            run_id: "private-execution-engine".to_owned().try_into().unwrap(),
            input_ref,
            project: serde_json::from_value(v["project"].clone()).unwrap(),
            scope: scenario.scope.clone(),
            scenario,
            definitions: serde_json::from_value(v["definitions"].clone()).unwrap(),
        };
        let identity: ExecutionIdentity = serde_json::from_value(
            json!({"session_id":"owner-session","execution_epoch_id":"owner-epoch",
            "project_id":v["project"]["id"],"host_context_session_id":"native-host"}),
        )
        .unwrap();
        let policy:ExecutionPolicy=serde_json::from_value(json!({"clock_mode":"host_monotonic","report_cutoff_ms":cutoff,
            "stop_new_dispatch_on_cutoff":true,"finish_policy":"drain_admitted_then_finish","target_quantity":{"value":{"coefficient":6,"decimal_places":0},"unit":"piece"},
            "source_prediction_window":{"start_ms":0,"end_ms":1200000},"observation_max_age_ms":2000})).unwrap();
        let candidates:Vec<CandidateBinding>=[("L1-D01",10000),("L1-D02",15000)].into_iter().map(|(device,ms)|serde_json::from_value(json!({
            "binding_id":format!("binding-{device}"),"device_id":device,"device_module_ref":{"id":"p201.b60.station","version":"1.0.0"},
            "operation_ref":{"id":"p201.b60.station","version":"1.0.0"},"execution_capacity_ref":{"kind":"resource","id":format!("{device}-CAP")},
            "input_ports":[{"role_id":"workpiece","port_id":"input","location_id":"L1-SOURCE","item_type_id":"benchmark-item-v1","unit_id":"piece"}],
            "output_ports":[{"role_id":"workpiece","port_id":"output","location_id":"L1-SINK","item_type_id":"benchmark-item-v1","unit_id":"piece"}],
            "resources":[],"phase_id":"cycle","nominal_work_ms":ms})).unwrap()).collect();
        let frozen:ExecutionFrozenInputs=serde_json::from_value(json!({"document_kind":"execution_frozen_inputs",
            "contract_version":EXECUTION_CONTRACT_VERSION,"identity":identity,"project_input_ref":input.input_ref,
            "project":input.project,"scenario":input.scenario,"modules":[],"execution_definitions":input.definitions,
            "scope":input.scope,"candidate_bindings":candidates,"asset_bindings":[]})).unwrap();
        ExecutionSession::prepare(
            frozen,
            policy,
            "execution-owner-session".to_owned().try_into().unwrap(),
            id("private-cloud-config"),
        )
        .unwrap()
    }
    pub(crate) fn session() -> ExecutionSession {
        session_with(120000, false)
    }
    fn run(s: &mut ExecutionSession, start: Instant) {
        for dev in ["L1-D01", "L1-D02"] {
            s.register_boot(&id(dev), format!("boot-{dev}").try_into().unwrap(), start)
                .unwrap();
        }
        s.control(
            id("operator"),
            id("run"),
            s.revision,
            ControlAction::Run {},
            start,
        )
        .unwrap();
    }
    fn observation(s: &ExecutionSession, dev: &str, seq: u64) -> ExecutionTelemetryObservation {
        let slot = &s.devices[&id(dev)];
        serde_json::from_value(json!({"contract_version":EXECUTION_CONTRACT_VERSION,"session_id":s.identity.session_id,
            "execution_epoch_id":s.identity.execution_epoch_id,"device_id":dev,"device_boot_id":slot.boot,
            "device_sequence":seq,"device_timestamp":"device-private-clock","accept_new_work":true,"device_state":"idle","active_command_id":null,
            "dataset_id":s.dataset_id,"source_purpose":"execution","observation_probe_id":slot.probe.as_ref().unwrap().probe_id})).unwrap()
    }
    fn ready(s: &mut ExecutionSession, dev: &str, seq: u64, now: Instant) {
        s.tick(now).unwrap();
        let o = observation(s, dev, seq);
        let a = s
            .apply_observation(id(&format!("observation-{dev}-{seq}")), stamp(), o, now)
            .unwrap();
        assert!(matches!(a.decision, ReceiptAdmissionDecision::Applied));
    }
    fn receipt(command: &Command, seq: u64, kind: Value) -> DeviceReceipt {
        serde_json::from_value(json!({"session_id":command.identity.session_id,"execution_epoch_id":command.identity.execution_epoch_id,
            "command_id":command.attempt.command_id,"device_id":command.device_id,"phase_id":command.attempt.phase_id,
            "phase_token":command.attempt.phase_token,"device_boot_id":command.target_device_boot_id,"device_sequence":seq,
            "device_timestamp":"independent-device-time","receipt":kind})).unwrap()
    }
    fn commands(s: &ExecutionSession) -> Vec<Command> {
        s.cloud_work()
            .items
            .into_iter()
            .filter_map(|w| match w {
                CloudWorkItem::PublishCommand { command, .. } => Some(command),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn preparation_has_one_original_six_job_engine_and_no_clock_or_dispatch() {
        let s = session();
        let o = s.observe().unwrap();
        assert!(!o.clock_started);
        assert_eq!(o.logical_work.len(), 6);
        assert_eq!(o.items.len(), 6);
        assert!(o.jobs.iter().all(|j| j.started_at_ms.is_none()));
        assert!(o.allocations.is_empty());
        assert!(commands(&s).is_empty());
        assert_eq!(s.publications.len(), 1);
        assert!(!s.publications[&0].snapshot.coverage.clock_started);
        assert_eq!(s.elapsed(Instant::now()).unwrap(), 0);
        assert_eq!(
            s.devices[&id("L1-D01")].binding.nominal_work_ms.value(),
            10000
        );
        assert_eq!(
            s.devices[&id("L1-D02")].binding.nominal_work_ms.value(),
            15000
        );
    }
    #[test]
    fn original_probe_expiry_and_exact_retry_do_not_refresh_eligibility() {
        let mut s = session();
        let start = Instant::now();
        run(&mut s, start);
        let old = observation(&s, "L1-D01", 1);
        let revision = s.revision;
        s.tick(at(start, 2001)).unwrap();
        assert!(s.revision > revision);
        let a = s
            .apply_observation(id("late"), stamp(), old.clone(), at(start, 2001))
            .unwrap();
        assert!(matches!(a.decision, ReceiptAdmissionDecision::Rejected));
        assert!(commands(&s).is_empty());
        let r = s.revision;
        let count = s.admissions.len();
        let repeated = s
            .apply_observation(id("late"), stamp(), old, at(start, 2050))
            .unwrap();
        assert!(matches!(
            repeated.decision,
            ReceiptAdmissionDecision::Duplicate
        ));
        assert_eq!(s.revision, r);
        assert_eq!(s.admissions.len(), count);
        assert!(s.devices[&id("L1-D01")].latest.is_none());
        ready(&mut s, "L1-D01", 2, at(start, 2100));
        assert_eq!(commands(&s).len(), 1);
    }
    #[test]
    fn operator_disable_survives_fresh_device_acceptance_and_rank_uses_eligible_set() {
        let mut s = session();
        let start = Instant::now();
        run(&mut s, start);
        s.control(
            id("operator"),
            id("disable-A"),
            s.revision,
            ControlAction::SetDeviceAcceptance {
                device_id: id("L1-D01"),
                accept_new_work: false,
            },
            start,
        )
        .unwrap();
        ready(&mut s, "L1-D01", 1, at(start, 1));
        assert!(commands(&s).is_empty());
        ready(&mut s, "L1-D02", 1, at(start, 2));
        let c = commands(&s);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].device_id, id("L1-D02"));
        let explanation = &s.attempts[&c[0].attempt.command_id].dispatch;
        assert_eq!(
            explanation
                .candidates
                .iter()
                .find(|v| v.eligible)
                .unwrap()
                .rank
                .as_ref()
                .unwrap()
                .value(),
            1
        );
        assert!(
            !s.device_views(2)
                .iter()
                .find(|v| v.binding.device_id == id("L1-D01"))
                .unwrap()
                .operator_accept_new_work
        );
    }
    #[test]
    fn publication_barrier_preserves_unknown_work_until_positive_cloud_withdrawal() {
        let mut s = session();
        let start = Instant::now();
        run(&mut s, start);
        ready(&mut s, "L1-D01", 1, at(start, 1));
        let original = commands(&s)[0].clone();
        ready(&mut s, "L1-D02", 1, at(start, 2));
        s.control(
            id("operator"),
            id("disable-A"),
            s.revision,
            ControlAction::SetDeviceAcceptance {
                device_id: id("L1-D01"),
                accept_new_work: false,
            },
            at(start, 3),
        )
        .unwrap();
        let work = s.cloud_work();
        let (work_id, cid) = work
            .items
            .iter()
            .find_map(|w| match w {
                CloudWorkItem::WithdrawCommand {
                    work_id,
                    command_id,
                    ..
                } => Some((work_id.clone(), command_id.clone())),
                _ => None,
            })
            .unwrap();
        assert_eq!(cid, original.attempt.command_id);
        let still = s.observe().unwrap();
        assert_eq!(
            still
                .attempts
                .iter()
                .filter(|a| a.command.attempt.logical_job_id == original.attempt.logical_job_id)
                .count(),
            1
        );
        let mut wrong = serde_json::to_value(CloudEffectAck::WithdrawalResult {
            work_id: work_id.clone(),
            command_id: cid.clone(),
            state: CloudResponseVariant0ResultWithdrawalDataState::WithdrawnBeforeDelivery,
        })
        .unwrap();
        wrong["command_id"] = json!("foreign");
        assert!(
            s.acknowledge_cloud_work(serde_json::from_value(wrong).unwrap(), at(start, 4))
                .is_err()
        );
        s.acknowledge_cloud_work(
            CloudEffectAck::WithdrawalResult {
                work_id,
                command_id: cid,
                state: CloudResponseVariant0ResultWithdrawalDataState::WithdrawnBeforeDelivery,
            },
            at(start, 4),
        )
        .unwrap();
        let a = s.observe().unwrap();
        assert!(a.attempts.iter().any(|a| a.command.attempt.command_id
            == original.attempt.command_id
            && a.state == ExternalAttemptState::CancelledNotStarted));
        assert!(a.allocations.is_empty());
        assert!(a.acceptance_events.is_empty());
        // B still owns another published attempt. Only its own positive proof
        // frees it; then the original A work may acquire B with a new attempt.
        s.control(
            id("operator"),
            id("disable-B"),
            s.revision,
            ControlAction::SetDeviceAcceptance {
                device_id: id("L1-D02"),
                accept_new_work: false,
            },
            at(start, 5),
        )
        .unwrap();
        let (work_id, command_id) = s
            .cloud_work()
            .items
            .iter()
            .find_map(|w| match w {
                CloudWorkItem::WithdrawCommand {
                    work_id,
                    command_id,
                    ..
                } if *command_id != original.attempt.command_id => {
                    Some((work_id.clone(), command_id.clone()))
                }
                _ => None,
            })
            .unwrap();
        s.acknowledge_cloud_work(
            CloudEffectAck::WithdrawalResult {
                work_id,
                command_id,
                state: CloudResponseVariant0ResultWithdrawalDataState::WithdrawnBeforeDelivery,
            },
            at(start, 6),
        )
        .unwrap();
        s.control(
            id("operator"),
            id("enable-B"),
            s.revision,
            ControlAction::SetDeviceAcceptance {
                device_id: id("L1-D02"),
                accept_new_work: true,
            },
            at(start, 7),
        )
        .unwrap();
        let current = s.observe().unwrap();
        let reassigned = current
            .attempts
            .iter()
            .find(|a| {
                a.command.attempt.logical_job_id == original.attempt.logical_job_id
                    && a.state == ExternalAttemptState::Published
            })
            .unwrap();
        assert_eq!(reassigned.command.device_id, id("L1-D02"));
        assert_eq!(reassigned.command.attempt.attempt_ordinal.value(), 2);
        assert!(current.acceptance_events.is_empty());
        assert_eq!(current.items.len(), 6);
    }
    #[test]
    fn acknowledged_command_retry_accepts_the_servers_same_payload_storage_status() {
        let mut s = session();
        let start = Instant::now();
        run(&mut s, start);
        ready(&mut s, "L1-D01", 1, at(start, 1));
        let CloudWorkItem::PublishCommand { work_id, command } = s
            .cloud_work()
            .items
            .into_iter()
            .find(|w| matches!(w, CloudWorkItem::PublishCommand { .. }))
            .unwrap()
        else {
            panic!("command work")
        };
        let ack = CloudEffectAck::CommandStored {
            work_id: work_id.clone(),
            command_id: command.attempt.command_id.clone(),
            storage_status: CloudResponseVariant0ResultCommandStoredDataStorageStatus::Stored,
        };
        s.acknowledge_cloud_work(ack, start).unwrap();
        let revision = s.revision;
        s.acknowledge_cloud_work(CloudEffectAck::CommandStored{work_id,command_id:command.attempt.command_id,
            storage_status:CloudResponseVariant0ResultCommandStoredDataStorageStatus::AlreadyStoredSamePayload},start).unwrap();
        assert_eq!(s.revision, revision);
        assert!(commands(&s).is_empty());
        assert!(s.observe().unwrap().acceptance_events.is_empty());
    }
    #[test]
    fn completed_before_started_is_retained_then_applied_once_despite_newer_telemetry() {
        let mut s = session();
        let start = Instant::now();
        run(&mut s, start);
        ready(&mut s, "L1-D01", 1, at(start, 1));
        let command = commands(&s)[0].clone();
        let completed = receipt(
            &command,
            3,
            json!({"kind":"completed","device_reported_duration_ms":10000}),
        );
        let first = s
            .apply_receipt(id("done"), stamp(), completed.clone(), at(start, 10))
            .unwrap();
        assert!(matches!(
            first.decision,
            ReceiptAdmissionDecision::PendingDependency
        ));
        assert!(s.observe().unwrap().allocations.is_empty());
        let mut newer = observation(&s, "L1-D01", 20);
        newer.device_state = ExecutionTelemetryObservationDeviceState::Busy;
        newer.active_command_id = Some(command.attempt.command_id.clone());
        s.apply_observation(id("newer-telemetry"), stamp(), newer, at(start, 11))
            .unwrap();
        s.apply_receipt(
            id("started"),
            stamp(),
            receipt(&command, 2, json!({"kind":"started"})),
            at(start, 12),
        )
        .unwrap();
        let after = s.observe().unwrap();
        assert!(
            after.allocations.is_empty(),
            "Completed released the original active allocation"
        );
        assert_eq!(after.acceptance_events.len(), 1);
        assert!(matches!(
            s.messages[&id("done")].latest.decision,
            ReceiptAdmissionDecision::Applied
        ));
        assert_eq!(
            s.admissions
                .iter()
                .filter(|a| a.receipt_id == id("done"))
                .count(),
            2,
            "pending and later applied stay immutable"
        );
        s.apply_receipt(id("done"), stamp(), completed, at(start, 13))
            .unwrap();
        assert_eq!(s.observe().unwrap().acceptance_events.len(), 1);
        assert!(s.observe().unwrap().allocations.is_empty());
        assert!(
            matches!(s.phase, ExecutionSnapshotLifecycleState::Running),
            "resolved dependency restores original dispatch mode"
        );
        ready(&mut s, "L1-D01", 21, at(start, 14));
        assert_eq!(s.observe().unwrap().attempts.len(), 2);
    }
    #[test]
    fn pause_resume_preserves_the_original_clock_and_control_request_identity() {
        let mut s = session();
        let start = Instant::now();
        run(&mut s, start);
        let rev = s.revision;
        let ack = s
            .control(
                id("operator"),
                id("pause"),
                rev,
                ControlAction::PauseDispatch {},
                at(start, 100),
            )
            .unwrap();
        let repeated = s
            .control(
                id("operator"),
                id("pause"),
                rev,
                ControlAction::PauseDispatch {},
                at(start, 500),
            )
            .unwrap();
        assert!(equal(&ack, &repeated));
        assert!(
            s.control(
                id("another-operator"),
                id("pause"),
                rev,
                ControlAction::PauseDispatch {},
                at(start, 500)
            )
            .is_err(),
            "another caller cannot reuse the original request record"
        );
        assert!(
            s.control(
                id("operator"),
                id("pause"),
                rev,
                ControlAction::StopDispatch {},
                at(start, 500)
            )
            .is_err()
        );
        s.control(
            id("operator"),
            id("resume"),
            s.revision,
            ControlAction::ResumeDispatch {},
            at(start, 700),
        )
        .unwrap();
        assert_eq!(s.elapsed(at(start, 1200)).unwrap(), 1200);
        assert!(s.clock_anchor == Some(start));
        assert!(
            s.control(
                id("operator"),
                id("stale-revision"),
                0,
                ControlAction::StopDispatch {},
                at(start, 1200)
            )
            .is_err()
        );
    }
    #[test]
    fn actual_same_stock_accepts_exactly_six_units_and_never_uses_nominal_completion() {
        let mut s = session();
        let start = Instant::now();
        run(&mut s, start);
        for n in 0..6u64 {
            let time = n * 10001 + 1;
            ready(&mut s, "L1-D01", n * 3 + 1, at(start, time));
            let command = commands(&s).last().unwrap().clone();
            s.apply_receipt(
                id(&format!("start-{n}")),
                stamp(),
                receipt(&command, n * 3 + 2, json!({"kind":"started"})),
                at(start, time + 1),
            )
            .unwrap();
            assert_eq!(s.observe().unwrap().allocations.len(), 1);
            s.tick(at(start, time + 9999)).unwrap();
            assert_eq!(s.observe().unwrap().acceptance_events.len(), n as usize);
            s.apply_receipt(
                id(&format!("complete-{n}")),
                stamp(),
                receipt(
                    &command,
                    n * 3 + 3,
                    json!({"kind":"completed","device_reported_duration_ms":10000}),
                ),
                at(start, time + 10000),
            )
            .unwrap();
        }
        let result = s.observe().unwrap();
        assert_eq!(result.logical_work.len(), 6);
        assert_eq!(result.jobs.len(), 6);
        assert_eq!(result.acceptance_events.len(), 6);
        assert!(result.allocations.is_empty());
        assert!(result.goal_met && result.drained);
        assert_eq!(result.items.len(), 6);
        assert_eq!(
            result
                .items
                .iter()
                .map(|item| item.id.as_str().to_owned())
                .collect::<std::collections::BTreeSet<_>>(),
            (0..6).map(|n| format!("blank-{n}")).collect()
        );
        assert!(matches!(
            s.phase,
            ExecutionSnapshotLifecycleState::Completed
        ));
        let final_revision = s.revision;
        assert!(
            s.control(
                id("operator"),
                id("after-completed"),
                final_revision,
                ControlAction::SetDeviceAcceptance {
                    device_id: id("L1-D01"),
                    accept_new_work: false
                },
                at(start, 60008)
            )
            .is_err()
        );
        assert_eq!(s.revision, final_revision);
        let history = s.projector.history_document().unwrap();
        assert!(
            history
                .records
                .iter()
                .enumerate()
                .all(|(i, r)| r.record_sequence.value() == i as u64 + 1)
        );
        assert!(s.publications.keys().copied().eq(0..=s.revision));
        let frozen =
            serde_json::to_value(s.projector.snapshot(&SnapshotSelection::Latest {}).unwrap())
                .unwrap();
        let extra = observation(&s, "L1-D01", 99);
        assert!(
            s.apply_observation(
                id("after-terminal-telemetry"),
                stamp(),
                extra,
                at(start, 60010)
            )
            .is_err()
        );
        assert_eq!(
            serde_json::to_value(s.projector.snapshot(&SnapshotSelection::Latest {}).unwrap())
                .unwrap(),
            frozen
        );
        assert_eq!(
            result
                .acceptance_events
                .iter()
                .map(|a| a.quantity.value.coefficient.value())
                .sum::<u64>(),
            6
        );
    }
    #[test]
    fn same_millisecond_cutoff_keeps_started_prefix_before_pending_completion_resolution() {
        let mut s = session_with(12, false);
        let start = Instant::now();
        run(&mut s, start);
        ready(&mut s, "L1-D01", 1, at(start, 1));
        let command = commands(&s)[0].clone();
        s.apply_receipt(
            id("pending-done"),
            stamp(),
            receipt(
                &command,
                3,
                json!({"kind":"completed","device_reported_duration_ms":10000}),
            ),
            at(start, 10),
        )
        .unwrap();
        let started = s
            .apply_receipt(
                id("late-start"),
                stamp(),
                receipt(&command, 2, json!({"kind":"started"})),
                at(start, 12),
            )
            .unwrap();
        let cutoff = s
            .projector
            .snapshot(&SnapshotSelection::Cutoff {})
            .unwrap()
            .clone();
        let latest = s.projector.snapshot(&SnapshotSelection::Latest {}).unwrap();
        assert_eq!(
            cutoff.execution_revision.value(),
            started.execution_revision.value()
        );
        assert_eq!(cutoff.coverage.observed_until_host_ms.value(), 12);
        assert_eq!(cutoff.business_facts.acceptance_events.len(), 0);
        assert_eq!(latest.business_facts.acceptance_events.len(), 1);
        assert!(latest.execution_revision.value() > cutoff.execution_revision.value());
        let evidence = s
            .receipt_evidence
            .iter()
            .find(|e| {
                e.admission.receipt_id == id("pending-done")
                    && matches!(e.admission.decision, ReceiptAdmissionDecision::Applied)
            })
            .unwrap();
        assert_eq!(evidence.admission.host_received_elapsed_ms.value(), 10);
        assert_eq!(evidence.committed_at_host_ms.as_ref().unwrap().value(), 12);
        assert_eq!(
            cutoff
                .receipt_admissions
                .iter()
                .filter(|a| a.receipt_id == id("pending-done"))
                .count(),
            1
        );
        assert_eq!(
            latest
                .receipt_admissions
                .iter()
                .filter(|a| a.receipt_id == id("pending-done"))
                .count(),
            2
        );
        s.tick(at(start, 30)).unwrap();
        assert!(equal(
            s.projector.snapshot(&SnapshotSelection::Cutoff {}).unwrap(),
            &cutoff
        ));
        assert_eq!(
            s.publications
                .values()
                .filter(|p| p.as_cutoff_snapshot)
                .count(),
            1
        );
    }
    #[test]
    fn later_receipt_never_revises_the_original_clock_cutoff_or_closes_its_open_claim() {
        let mut s = session_with(10, false);
        let start = Instant::now();
        run(&mut s, start);
        ready(&mut s, "L1-D01", 1, at(start, 1));
        let command = commands(&s)[0].clone();
        s.apply_receipt(
            id("start"),
            stamp(),
            receipt(&command, 2, json!({"kind":"started"})),
            at(start, 2),
        )
        .unwrap();
        s.tick(at(start, 20)).unwrap();
        let cutoff = s
            .projector
            .snapshot(&SnapshotSelection::Cutoff {})
            .unwrap()
            .clone();
        assert_eq!(cutoff.coverage.observed_until_host_ms.value(), 10);
        assert!(cutoff.business_facts.acceptance_events.is_empty());
        assert!(cutoff.business_facts.resource_claims.iter().any(
            |c| c.open_interval_index.is_some()
                && matches!(
                    c.view_boundary,
                    ProjectedResourceClaimViewBoundary::ReportCutoff
                )
        ));
        s.apply_receipt(
            id("done"),
            stamp(),
            receipt(
                &command,
                3,
                json!({"kind":"completed","device_reported_duration_ms":10000}),
            ),
            at(start, 30),
        )
        .unwrap();
        assert_eq!(
            s.projector
                .snapshot(&SnapshotSelection::Latest {})
                .unwrap()
                .business_facts
                .acceptance_events
                .len(),
            1
        );
        assert!(equal(
            s.projector.snapshot(&SnapshotSelection::Cutoff {}).unwrap(),
            &cutoff
        ));
    }
    #[test]
    fn original_arrival_and_expiry_are_observed_without_backdating_new_probe_issuance() {
        let mut s = session_with(120000, true);
        let start = Instant::now();
        assert_eq!(s.publications[&0].snapshot.business_facts.items.len(), 5);
        run(&mut s, start);
        ready(&mut s, "L1-D01", 1, at(start, 1));
        s.tick(at(start, 2500)).unwrap();
        let arrival = s
            .publications
            .values()
            .find(|p| p.snapshot.coverage.observed_until_host_ms.value() == 5)
            .unwrap();
        assert_eq!(arrival.snapshot.business_facts.items.len(), 6);
        let expired = s
            .publications
            .values()
            .find(|p| p.snapshot.coverage.observed_until_host_ms.value() == 2001)
            .unwrap();
        let device = expired
            .snapshot
            .devices
            .iter()
            .find(|d| d.binding.device_id == id("L1-D01"))
            .unwrap();
        assert!(matches!(
            device.freshness,
            ExecutionDeviceViewFreshness::Stale
        ));
        assert_eq!(
            device
                .pending_observation_probe
                .as_ref()
                .unwrap()
                .issued_at_host_ms
                .value(),
            1
        );
        let latest = s.projector.snapshot(&SnapshotSelection::Latest {}).unwrap();
        let device = latest
            .devices
            .iter()
            .find(|d| d.binding.device_id == id("L1-D01"))
            .unwrap();
        assert_eq!(
            device
                .pending_observation_probe
                .as_ref()
                .unwrap()
                .issued_at_host_ms
                .value(),
            2500
        );
        assert_eq!(latest.pending_reservations.len(), 1);
        let history = s.projector.history_document().unwrap();
        assert!(history.records.iter().any(|r| r.at_host_ms.value() == 5
            && matches!(&r.object,ExecutionObservedObject::Item{data} if data.id==id("blank-5"))));
    }
    #[test]
    fn failed_after_started_keeps_its_actual_item_and_resource_and_cannot_be_reassigned() {
        let mut s = session();
        let start = Instant::now();
        run(&mut s, start);
        ready(&mut s, "L1-D01", 1, at(start, 1));
        let command = commands(&s)[0].clone();
        s.apply_receipt(
            id("started"),
            stamp(),
            receipt(&command, 2, json!({"kind":"started"})),
            at(start, 2),
        )
        .unwrap();
        s.apply_receipt(
            id("failed"),
            stamp(),
            receipt(
                &command,
                3,
                json!({"kind":"failed","code":"motor","message":"actual fault"}),
            ),
            at(start, 3),
        )
        .unwrap();
        let original = s.observe().unwrap();
        assert!(!original.drained);
        assert!(!original.resource_claims.is_empty());
        assert!(matches!(
            s.phase,
            ExecutionSnapshotLifecycleState::NeedsResolution
        ));
        assert!(
            s.engine
                .confirm_external_withdrawal(&command.attempt.command_id, uint(4))
                .is_err()
        );
        assert!(s.observe().unwrap().acceptance_events.is_empty());
        assert!(
            s.device_views(3)
                .iter()
                .find(|v| v.binding.device_id == id("L1-D01"))
                .unwrap()
                .blocking_reasons
                .iter()
                .any(|r| r.as_str() == "original_attempt_unsettled")
        );
    }
}
fn engine_error(e: sf_simulation::ExternalError) -> ApiError {
    fault(ErrorCode::InvalidInput, e.to_string())
}
fn equal(a: &impl serde::Serialize, b: &impl serde::Serialize) -> bool {
    serde_json::to_value(a).unwrap() == serde_json::to_value(b).unwrap()
}

pub(crate) struct DeviceSlot {
    pub binding: CandidateBinding,
    pub operator_accept: bool,
    pub boot: Option<DeviceBootId>,
    pub probe: Option<ObservationProbe>,
    pub latest: Option<HostObservedDevice>,
}
#[derive(Clone)]
pub(crate) struct AttemptLedger {
    pub command: Command,
    pub reserved_at_host_ms: u64,
    pub published_at_host_ms: u64,
    pub dispatch: DispatchExplanation,
    pub reservation_id: ObjectId,
    pub resource_plan: Vec<ResourceUnits>,
}
#[derive(Clone)]
pub(crate) struct AdmittedMessage {
    pub payload: Value,
    pub production_receipt: Option<DeviceReceipt>,
    pub latest: ReceiptAdmission,
    pub first_received_elapsed_ms: u64,
}

/// Preparation transfers this single owner into ProjectHost after its actual
/// initial cloud publication. No public request can supply an Engine or clock.
#[derive(Clone, Copy)]
enum DispatchMode {
    Prepared,
    Running,
    Paused,
    Stopped,
    Reconciliation,
}

pub(crate) struct ExecutionSession {
    dispatch_mode: DispatchMode,
    pub engine: Engine,
    pub frozen: ExecutionFrozenInputs,
    pub provenance: ExecutionSnapshotProvenance,
    pub projector: ExecutionProjector,
    pub receipt_evidence: Vec<HostReceiptEvidence>,
    pub identity: ExecutionIdentity,
    pub policy: ExecutionPolicy,
    pub dataset_id: Option<CloudDatasetId>,
    pub revision: u64,
    pub phase: ExecutionSnapshotLifecycleState,
    pub clock_anchor: Option<Instant>,
    pub devices: BTreeMap<ObjectId, DeviceSlot>,
    pub attempts: BTreeMap<DispatchCommandId, AttemptLedger>,
    pub dispatch_decisions: BTreeMap<ObjectId, DispatchExplanation>,
    pub admissions: Vec<ReceiptAdmission>,
    pub messages: BTreeMap<ObjectId, AdmittedMessage>,
    pub cloud_published_revision: Option<u64>,
    pub pending_cloud_work: Vec<CloudWorkItem>,
    pub publications: BTreeMap<u64, SnapshotPublication>,
    control_requests: BTreeMap<(ObjectId, ObjectId), (Value, ControlAck)>,
    acknowledged_work: BTreeMap<ObjectId, Value>,
    device_sequences: BTreeMap<(ObjectId, DeviceBootId, u64), ObjectId>,
    next_identity: u64,
    admission_sequence: u64,
}

impl ExecutionSession {
    pub fn prepare(
        frozen: ExecutionFrozenInputs,
        policy: ExecutionPolicy,
        dataset_id: CloudDatasetId,
        connection_ref: ObjectId,
    ) -> Result<Self> {
        Self::prepare_inner(frozen, policy, Some(dataset_id), connection_ref)
    }
    pub fn prepare_before_cloud(
        frozen: ExecutionFrozenInputs,
        policy: ExecutionPolicy,
        connection_ref: ObjectId,
    ) -> Result<Self> {
        Self::prepare_inner(frozen, policy, None, connection_ref)
    }
    fn prepare_inner(
        frozen: ExecutionFrozenInputs,
        policy: ExecutionPolicy,
        dataset_id: Option<CloudDatasetId>,
        connection_ref: ObjectId,
    ) -> Result<Self> {
        let identity = frozen.identity.clone();
        let candidates = frozen.candidate_bindings.clone();
        let device_count = candidates
            .iter()
            .map(|b| b.device_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        if device_count != candidates.len() {
            return Err(fault(
                ErrorCode::InvalidInput,
                "One frozen binding per device is required for this execution",
            ));
        }
        let input = EngineInput {
            run_id: RunId::try_from(format!("{}:engine", identity.session_id.as_str())).unwrap(),
            input_ref: frozen.project_input_ref.clone(),
            project: frozen.project.clone(),
            scenario: frozen.scenario.clone(),
            definitions: frozen.execution_definitions.clone(),
            scope: frozen.scope.clone(),
        };
        let engine =
            Engine::new_external(input, identity.clone(), policy.clone(), candidates.clone())
                .map_err(engine_error)?;
        let provenance = ExecutionSnapshotProvenance {
            mode: ExecutionSnapshotProvenanceMode::SoftwareInLoop,
            cloud_connection_ref: connection_ref,
            device_binding_refs: candidates.iter().map(|b| b.binding_id.clone()).collect(),
            producer: ExecutionSnapshotProvenanceProducer::Host,
        };
        let devices = candidates
            .into_iter()
            .map(|binding| {
                (
                    binding.device_id.clone(),
                    DeviceSlot {
                        binding,
                        operator_accept: true,
                        boot: None,
                        probe: None,
                        latest: None,
                    },
                )
            })
            .collect();
        let mut session = Self {
            engine,
            frozen,
            provenance,
            projector: ExecutionProjector::new(),
            receipt_evidence: vec![],
            identity,
            policy,
            devices,
            dataset_id,
            revision: 0,
            phase: ExecutionSnapshotLifecycleState::Prepared,
            dispatch_mode: DispatchMode::Prepared,
            clock_anchor: None,
            attempts: BTreeMap::new(),
            dispatch_decisions: BTreeMap::new(),
            admissions: vec![],
            messages: BTreeMap::new(),
            cloud_published_revision: None,
            pending_cloud_work: vec![],
            publications: BTreeMap::new(),
            control_requests: BTreeMap::new(),
            acknowledged_work: BTreeMap::new(),
            device_sequences: BTreeMap::new(),
            next_identity: 1,
            admission_sequence: 0,
        };
        session.capture_frame()?;
        Ok(session)
    }
    fn is_terminal(&self) -> bool {
        matches!(
            self.phase,
            ExecutionSnapshotLifecycleState::Completed
                | ExecutionSnapshotLifecycleState::Stopped
                | ExecutionSnapshotLifecycleState::Failed
        )
    }
    fn has_cutoff_frame(&self) -> bool {
        self.publications.values().any(|p| p.as_cutoff_snapshot)
    }
    fn capture_frame(&mut self) -> Result<()> {
        let observation = self.observe()?;
        let as_cutoff =
            self.engine.external_cutoff_observation().is_some() && !self.has_cutoff_frame();
        if as_cutoff && observation.elapsed_ms != self.policy.report_cutoff_ms.value() {
            return Err(fault(
                ErrorCode::ComputationFailed,
                "Original Host cutoff boundary was not captured; later state cannot backfill it",
            ));
        }
        let attempts = self
            .attempts
            .values()
            .map(|m| {
                let actual = observation
                    .attempts
                    .iter()
                    .find(|a| a.command.attempt.command_id == m.command.attempt.command_id)
                    .expect("same original Engine attempt ledger");
                AttemptMetadata {
                    command_id: m.command.attempt.command_id.clone(),
                    reservation_id: m.reservation_id.clone(),
                    reserved_at_host_ms: Some(uint(m.reserved_at_host_ms)),
                    published_at_host_ms: Some(uint(m.published_at_host_ms)),
                    withdrawn_unpublished_at_host_ms: None,
                    reserved_resource_units: m.resource_plan.clone(),
                    pending_input_ordinals: Some(actual.pending_input_ordinals.clone()),
                    dispatch_decision: Some(m.dispatch.clone()),
                }
            })
            .collect();
        let ledger = HostProjectionLedger {
            execution_revision: uint(self.revision),
            host_admission_sequence: uint(self.admission_sequence),
            policy: self.policy.clone(),
            provenance: self.provenance.clone(),
            lifecycle_state: self.phase.clone(),
            lifecycle_reason: matches!(
                self.phase,
                ExecutionSnapshotLifecycleState::NeedsResolution
            )
            .then(|| {
                DeviceText::try_from("Original execution facts require reconciliation".to_owned())
                    .unwrap()
            }),
            new_dispatch_allowed: matches!(self.phase, ExecutionSnapshotLifecycleState::Running)
                && !observation.dispatch_stopped
                && !observation.drained,
            terminal_disposition_recorded: false,
            attempts,
            receipts: self.receipt_evidence.clone(),
            observation_admissions: self
                .admissions
                .iter()
                .filter(|a| {
                    self.messages
                        .get(&a.receipt_id)
                        .is_some_and(|m| m.production_receipt.is_none())
                })
                .cloned()
                .collect(),
            devices: self.device_views(observation.elapsed_ms),
            queued_dispatch_decisions: self.dispatch_decisions.clone(),
            missing_conditions: vec![],
        };
        let batch = self
            .projector
            .append(ProjectionInput {
                observation: &observation,
                frozen: &self.frozen,
                ledger: &ledger,
                as_cutoff_snapshot: as_cutoff,
                history_contiguous: true,
            })
            .map_err(|e| fault(ErrorCode::ComputationFailed, e.to_string()))?;
        self.queue_publication(SnapshotPublication {
            contract_version: SnapshotPublicationContractVersion::SfExecution20Draft11,
            expected_published_revision: if self.revision == 0 {
                None
            } else {
                Some(uint(self.revision - 1))
            },
            snapshot: batch.snapshot,
            as_cutoff_snapshot: as_cutoff,
            observation_records: batch.observation_records,
        })
    }
    /// Observe deterministic original model/expiry/cutoff boundaries before a
    /// later input. New probes/dispatch still use the actual input time.
    fn advance_boundaries(&mut self, elapsed: u64, receipt_at_exact_cutoff: bool) -> Result<()> {
        if self.clock_anchor.is_none() || self.is_terminal() {
            return Ok(());
        }
        loop {
            let current = self.observe()?.elapsed_ms;
            let mut boundaries = vec![];
            if let Some(next) = self
                .engine
                .next_external_boundary_ms()
                .map_err(engine_error)?
            {
                if next.value() > current && next.value() < elapsed {
                    boundaries.push(next.value());
                }
            }
            let cutoff = self.policy.report_cutoff_ms.value();
            if !self.has_cutoff_frame()
                && cutoff >= current
                && (cutoff < elapsed || cutoff == elapsed && !receipt_at_exact_cutoff)
            {
                boundaries.push(cutoff);
            }
            for slot in self.devices.values() {
                if let Some(latest) = &slot.latest {
                    if let Some(expiry) = latest
                        .observation_probe
                        .expires_at_host_ms
                        .value()
                        .checked_add(1)
                    {
                        if current < expiry && expiry < elapsed {
                            boundaries.push(expiry);
                        }
                    }
                }
            }
            let Some(next) = boundaries.into_iter().min() else {
                break;
            };
            let revision = self.next_revision()?;
            self.engine
                .advance_external(uint(next))
                .map_err(engine_error)?;
            self.revision = revision;
            self.reconcile_lifecycle()?;
            self.capture_frame()?;
            if self.is_terminal() {
                break;
            }
        }
        Ok(())
    }
    pub fn observe(&self) -> Result<ExternalObservation> {
        self.engine.external_observation().map_err(engine_error)
    }
    pub fn elapsed(&self, now: Instant) -> Result<u64> {
        let Some(anchor) = self.clock_anchor else {
            return Ok(0);
        };
        let millis = now
            .checked_duration_since(anchor)
            .ok_or_else(|| {
                fault(
                    ErrorCode::InvalidInput,
                    "Host monotonic time cannot precede its original run anchor",
                )
            })?
            .as_millis();
        if millis > MAX_SAFE_UINT as u128 {
            return Err(fault(
                ErrorCode::InvalidInput,
                "Host elapsed exceeds the execution wire range",
            ));
        }
        Ok(millis as u64)
    }
    fn next_revision(&self) -> Result<u64> {
        self.revision
            .checked_add(1)
            .filter(|n| *n <= MAX_SAFE_UINT)
            .ok_or_else(|| fault(ErrorCode::InvalidInput, "Execution revision is exhausted"))
    }
    fn next_id(&mut self, purpose: &str) -> ObjectId {
        let id = object(format!(
            "{}:{purpose}:{}",
            self.identity.session_id.as_str(),
            self.next_identity
        ));
        self.next_identity += 1;
        id
    }
    pub fn register_boot(
        &mut self,
        device: &ObjectId,
        boot: DeviceBootId,
        now: Instant,
    ) -> Result<()> {
        let slot = self.devices.get(device).ok_or_else(|| {
            fault(
                ErrorCode::PermissionDenied,
                "Device is outside this execution",
            )
        })?;
        if let Some(original) = &slot.boot {
            return if original == &boot {
                Ok(())
            } else {
                Err(fault(
                    ErrorCode::InvalidInput,
                    "The original device boot must be settled and retired before replacement",
                ))
            };
        }
        if self.is_terminal() {
            return Err(fault(
                ErrorCode::InvalidInput,
                "Settled execution cannot admit a new device boot",
            ));
        }
        let elapsed = self.elapsed(now)?;
        self.advance_boundaries(elapsed, false)?;
        if self.is_terminal() {
            return Err(fault(
                ErrorCode::InvalidInput,
                "Execution settled before boot admission",
            ));
        }
        let next = self.next_revision()?;
        if self.clock_anchor.is_some() {
            self.engine
                .advance_external(uint(elapsed))
                .map_err(engine_error)?;
        }
        self.devices.get_mut(device).unwrap().boot = Some(boot);
        self.revision = next;
        self.refresh_probes(elapsed)?;
        self.reconcile_lifecycle()?;
        self.capture_frame()
    }
    fn refresh_probes(&mut self, elapsed: u64) -> Result<bool> {
        if self.clock_anchor.is_none()
            || matches!(
                self.phase,
                ExecutionSnapshotLifecycleState::Completed
                    | ExecutionSnapshotLifecycleState::Stopped
                    | ExecutionSnapshotLifecycleState::Failed
            )
        {
            return Ok(false);
        }
        let expires = elapsed
            .checked_add(self.policy.observation_max_age_ms.value())
            .filter(|n| *n <= MAX_SAFE_UINT)
            .ok_or_else(|| {
                fault(
                    ErrorCode::InvalidInput,
                    "Probe deadline exceeds the execution wire range",
                )
            })?;
        let devices: Vec<_> = self
            .devices
            .iter()
            .filter_map(|(id, s)| {
                (s.boot.is_some()
                    && s.probe
                        .as_ref()
                        .is_none_or(|p| p.expires_at_host_ms.value() < elapsed))
                .then_some(id.clone())
            })
            .collect();
        for device in &devices {
            let probe_id =
                ObservationProbeId::try_from(self.next_id("probe").as_str().to_owned()).unwrap();
            let slot = self.devices.get_mut(device).unwrap();
            slot.probe = Some(ObservationProbe {
                probe_id,
                device_id: device.clone(),
                device_boot_id: slot.boot.clone().unwrap(),
                issued_execution_revision: uint(self.revision),
                issued_at_host_ms: uint(elapsed),
                expires_at_host_ms: uint(expires),
            });
        }
        Ok(!devices.is_empty())
    }
    pub fn device_views(&self, elapsed: u64) -> Vec<ExecutionDeviceView> {
        self.devices
            .values()
            .map(|slot| {
                let fresh = slot
                    .latest
                    .as_ref()
                    .is_some_and(|v| elapsed <= v.observation_probe.expires_at_host_ms.value());
                let mut reasons = vec![];
                if !slot.operator_accept {
                    reasons.push("operator_disabled");
                }
                if slot.boot.is_none() {
                    reasons.push("device_boot_not_registered");
                }
                if !fresh {
                    reasons.push("fresh_observation_required");
                }
                if let Some(view) = &slot.latest {
                    if !view.observation.accept_new_work {
                        reasons.push("device_disabled_acceptance");
                    }
                    if !matches!(
                        view.observation.device_state,
                        ExecutionTelemetryObservationDeviceState::Idle
                    ) || view.observation.active_command_id.is_some()
                    {
                        reasons.push("device_not_idle");
                    }
                }
                let busy = self
                    .engine
                    .external_observation()
                    .expect("owned external Engine")
                    .attempts
                    .iter()
                    .any(|a| {
                        a.command.device_id == slot.binding.device_id
                            && matches!(
                                a.state,
                                ExternalAttemptState::Reserved
                                    | ExternalAttemptState::Published
                                    | ExternalAttemptState::Accepted
                                    | ExternalAttemptState::Started
                                    | ExternalAttemptState::ReconciliationRequired
                                    | ExternalAttemptState::Failed
                            )
                    });
                if busy {
                    reasons.push("original_attempt_unsettled");
                }
                ExecutionDeviceView {
                    binding: slot.binding.clone(),
                    operator_accept_new_work: slot.operator_accept,
                    latest_observation: slot.latest.clone(),
                    freshness: if slot.latest.is_none() {
                        ExecutionDeviceViewFreshness::Unknown
                    } else if fresh {
                        ExecutionDeviceViewFreshness::Fresh
                    } else {
                        ExecutionDeviceViewFreshness::Stale
                    },
                    effective_accept_new_work: if fresh {
                        Some(reasons.is_empty())
                    } else {
                        None
                    },
                    blocking_reasons: reasons
                        .into_iter()
                        .map(|s| DeviceText::try_from(s.to_owned()).unwrap())
                        .collect(),
                    registered_device_boot_id: slot.boot.clone(),
                    pending_observation_probe: slot.probe.clone(),
                }
            })
            .collect()
    }
    fn reconcile_lifecycle(&mut self) -> Result<()> {
        let mut obs = self.observe()?;
        if obs.goal_met && !obs.dispatch_stopped {
            self.engine.cancel();
            obs = self.observe()?;
        }
        if obs.attempts.iter().any(|a| {
            matches!(
                a.state,
                ExternalAttemptState::Failed | ExternalAttemptState::ReconciliationRequired
            ) || !a.unapplied_receipts.is_empty()
        }) {
            self.phase = ExecutionSnapshotLifecycleState::NeedsResolution;
        } else if obs.drained {
            self.phase = if obs.goal_met {
                ExecutionSnapshotLifecycleState::Completed
            } else {
                ExecutionSnapshotLifecycleState::Stopped
            };
        } else if obs.dispatch_stopped {
            self.phase = ExecutionSnapshotLifecycleState::Draining;
        } else {
            self.phase = match self.dispatch_mode {
                DispatchMode::Prepared => ExecutionSnapshotLifecycleState::Prepared,
                DispatchMode::Running => ExecutionSnapshotLifecycleState::Running,
                DispatchMode::Paused => ExecutionSnapshotLifecycleState::DispatchPaused,
                DispatchMode::Stopped => ExecutionSnapshotLifecycleState::Draining,
                DispatchMode::Reconciliation => ExecutionSnapshotLifecycleState::NeedsResolution,
            };
        }
        Ok(())
    }
    pub fn control(
        &mut self,
        caller_session: ObjectId,
        request_id: ObjectId,
        expected_revision: u64,
        action: ControlAction,
        now: Instant,
    ) -> Result<ControlAck> {
        let request_key = (caller_session, request_id);
        let payload =
            serde_json::json!({"expected_execution_revision":expected_revision,"action":action});
        if let Some((old, ack)) = self.control_requests.get(&request_key) {
            return if old == &payload {
                Ok(ack.clone())
            } else {
                Err(fault(
                    ErrorCode::InvalidInput,
                    "Original control request has another payload",
                ))
            };
        }
        if expected_revision != self.revision {
            let mut error = fault(
                ErrorCode::RevisionConflict,
                "Execution revision changed before control admission",
            );
            error.expected_revision = Some(Revision(uint(expected_revision)));
            error.actual_revision = Some(Revision(uint(self.revision)));
            return Err(error);
        }
        if matches!(
            self.phase,
            ExecutionSnapshotLifecycleState::Completed
                | ExecutionSnapshotLifecycleState::Stopped
                | ExecutionSnapshotLifecycleState::Failed
        ) {
            return Err(fault(
                ErrorCode::InvalidInput,
                "Settled executions have no production controls",
            ));
        }
        let elapsed = self.elapsed(now)?;
        self.advance_boundaries(elapsed, false)?;
        if self.is_terminal() {
            return Err(fault(
                ErrorCode::InvalidInput,
                "Execution reached its terminal boundary before control",
            ));
        }
        let next = self.next_revision()?;
        if self.clock_anchor.is_some() {
            self.engine
                .advance_external(uint(elapsed))
                .map_err(engine_error)?;
        }
        let applied = match &action {
            ControlAction::Run {} => {
                if !matches!(self.dispatch_mode, DispatchMode::Prepared) {
                    return Err(fault(
                        ErrorCode::InvalidInput,
                        "Run requires the prepared original session",
                    ));
                }
                self.engine.activate_external().map_err(engine_error)?;
                self.clock_anchor = Some(now);
                self.dispatch_mode = DispatchMode::Running;
                self.phase = ExecutionSnapshotLifecycleState::Running;
                true
            }
            ControlAction::PauseDispatch {} => {
                if matches!(self.dispatch_mode, DispatchMode::Paused) {
                    false
                } else if matches!(self.dispatch_mode, DispatchMode::Running) {
                    self.engine.pause();
                    self.dispatch_mode = DispatchMode::Paused;
                    self.phase = ExecutionSnapshotLifecycleState::DispatchPaused;
                    true
                } else {
                    return Err(fault(
                        ErrorCode::InvalidInput,
                        "Pause requires a running execution",
                    ));
                }
            }
            ControlAction::ResumeDispatch {} => {
                if matches!(self.dispatch_mode, DispatchMode::Running) {
                    false
                } else if matches!(
                    self.dispatch_mode,
                    DispatchMode::Paused | DispatchMode::Reconciliation
                ) {
                    if self.observe()?.attempts.iter().any(|a| {
                        matches!(
                            a.state,
                            ExternalAttemptState::Failed
                                | ExternalAttemptState::ReconciliationRequired
                        )
                    }) {
                        return Err(fault(
                            ErrorCode::InvalidInput,
                            "Original unresolved execution facts must be reconciled before resume",
                        ));
                    }
                    self.engine.resume();
                    self.dispatch_mode = DispatchMode::Running;
                    self.phase = ExecutionSnapshotLifecycleState::Running;
                    true
                } else {
                    return Err(fault(
                        ErrorCode::InvalidInput,
                        "Resume requires dispatch_paused",
                    ));
                }
            }
            ControlAction::StopDispatch {} => {
                if self.clock_anchor.is_none() {
                    self.engine.cancel();
                    self.dispatch_mode = DispatchMode::Stopped;
                    self.phase = ExecutionSnapshotLifecycleState::Stopped;
                    true
                } else {
                    self.engine.cancel();
                    self.dispatch_mode = DispatchMode::Stopped;
                    self.phase = ExecutionSnapshotLifecycleState::Draining;
                    true
                }
            }
            ControlAction::RequestReconciliation {} => {
                if self.clock_anchor.is_none() {
                    return Err(fault(
                        ErrorCode::InvalidInput,
                        "Reconciliation requires an activated execution",
                    ));
                }
                self.engine.pause();
                self.dispatch_mode = DispatchMode::Reconciliation;
                self.phase = ExecutionSnapshotLifecycleState::NeedsResolution;
                true
            }
            ControlAction::SetDeviceAcceptance {
                device_id,
                accept_new_work,
            } => {
                let slot = self.devices.get_mut(device_id).ok_or_else(|| {
                    fault(
                        ErrorCode::PermissionDenied,
                        "Device is outside this execution",
                    )
                })?;
                let changed = slot.operator_accept != *accept_new_work;
                slot.operator_accept = *accept_new_work;
                changed
            }
        };
        self.revision = next;
        if self.clock_anchor.is_some() {
            self.engine
                .advance_external(uint(elapsed))
                .map_err(engine_error)?;
            self.reconcile_lifecycle()?;
            self.refresh_probes(elapsed)?;
            self.withdraw_disabled_commands()?;
            self.schedule(elapsed)?;
            self.reconcile_lifecycle()?;
        }
        self.capture_frame()?;
        let ack = ControlAck {
            identity: self.identity.clone(),
            execution_revision: uint(self.revision),
            action,
            admission: if applied {
                ControlAdmission::Applied
            } else {
                ControlAdmission::AlreadyApplied
            },
        };
        self.control_requests
            .insert(request_key, (payload, ack.clone()));
        Ok(ack)
    }
    fn withdraw_disabled_commands(&mut self) -> Result<()> {
        for attempt in self.observe()?.attempts {
            let disabled = !self.devices[&attempt.command.device_id].operator_accept
                || matches!(self.phase, ExecutionSnapshotLifecycleState::Draining);
            let command_id = &attempt.command.attempt.command_id;
            if disabled && matches!(attempt.state, ExternalAttemptState::Published | ExternalAttemptState::Accepted)
                && !self.pending_cloud_work.iter().any(|w| matches!(w,CloudWorkItem::WithdrawCommand { command_id:id,.. } if id==command_id)) {
                let work_id = self.next_id("withdraw");
                self.pending_cloud_work.push(CloudWorkItem::WithdrawCommand { work_id,
                    identity: self.identity.clone(), command_id: command_id.clone() });
            }
        }
        Ok(())
    }
    fn schedule(&mut self, elapsed: u64) -> Result<()> {
        if !matches!(self.phase, ExecutionSnapshotLifecycleState::Running) {
            return Ok(());
        }
        let original = self.observe()?;
        if original.dispatch_stopped || original.drained {
            return Ok(());
        }
        for logical_job_id in original.queued_logical_jobs {
            let current = self.observe()?;
            if current.attempts.iter().any(|a| {
                a.command.attempt.logical_job_id == logical_job_id
                    && !matches!(
                        a.state,
                        ExternalAttemptState::CancelledNotStarted
                            | ExternalAttemptState::WithdrawnUnpublished
                    )
            }) {
                continue;
            }
            let mut views = self.device_views(elapsed);
            views.sort_by(|a, b| {
                a.binding
                    .nominal_work_ms
                    .value()
                    .cmp(&b.binding.nominal_work_ms.value())
                    .then(a.binding.binding_id.cmp(&b.binding.binding_id))
            });
            let mut candidates = vec![];
            let mut selected = None;
            let ordinal = current
                .attempts
                .iter()
                .filter(|a| a.command.attempt.logical_job_id == logical_job_id)
                .map(|a| a.command.attempt.attempt_ordinal.value())
                .max()
                .unwrap_or(0)
                + 1;
            let mut eligible_rank = 0;
            for view in &views {
                let eligible = view.effective_accept_new_work == Some(true);
                if eligible {
                    eligible_rank += 1;
                }
                let mut candidate = DispatchCandidateView {
                    binding_id: view.binding.binding_id.clone(),
                    eligible,
                    blocking_reasons: view.blocking_reasons.clone(),
                    estimated_completion_host_ms: if eligible {
                        elapsed
                            .checked_add(view.binding.nominal_work_ms.value())
                            .filter(|n| *n <= MAX_SAFE_UINT)
                            .map(uint)
                    } else {
                        None
                    },
                    rank: if eligible {
                        Some(PositiveSafeUint::try_from(eligible_rank as u64).unwrap())
                    } else {
                        None
                    },
                };
                if eligible && selected.is_none() {
                    let command_id =
                        DispatchCommandId::try_from(self.next_id("command").as_str().to_owned())
                            .unwrap();
                    let attempt = AttemptRef {
                        logical_job_id: logical_job_id.clone(),
                        phase_id: view.binding.phase_id.clone(),
                        attempt_ordinal: SafeUint::try_from(ordinal)
                            .map_err(|s| fault(ErrorCode::InvalidInput, format!("{s:?}")))?,
                        command_id: command_id.clone(),
                        selected_binding_id: view.binding.binding_id.clone(),
                        phase_token: self.next_id("phase"),
                    };
                    match self.engine.prepare_external(
                        attempt,
                        view.registered_device_boot_id.clone().unwrap(),
                        uint(elapsed),
                    ) {
                        Ok(command) => {
                            // The command becomes non-reassignable before it can leave this owner.
                            self.engine
                                .mark_external_published(&command_id, uint(elapsed))
                                .map_err(engine_error)?;
                            selected = Some(command);
                        }
                        Err(e)
                            if matches!(
                                e.code,
                                "external.resources" | "external.material" | "external.phase"
                            ) =>
                        {
                            eligible_rank -= 1;
                            candidate.eligible = false;
                            candidate.rank = None;
                            candidate.estimated_completion_host_ms = None;
                            candidate
                                .blocking_reasons
                                .push(DeviceText::try_from(e.code.to_owned()).unwrap());
                        }
                        Err(e) => return Err(engine_error(e)),
                    }
                }
                candidates.push(candidate);
            }
            let explanation = DispatchExplanation {
                based_on_host_admission_sequence: uint(self.admission_sequence),
                decision_host_ms: uint(elapsed),
                reason: if selected.is_none() {
                    DispatchExplanationReason::WaitingForEligibleBinding
                } else if ordinal > 1 {
                    DispatchExplanationReason::ReselectAfterConfirmedWithdrawal
                } else if views.len() > 1
                    && views[0].binding.nominal_work_ms.value()
                        == views[1].binding.nominal_work_ms.value()
                {
                    DispatchExplanationReason::StableBindingTiebreak
                } else {
                    DispatchExplanationReason::EarliestEstimatedCompletion
                },
                selected_binding_id: selected
                    .as_ref()
                    .map(|c| c.attempt.selected_binding_id.clone()),
                candidates,
            };
            self.dispatch_decisions
                .insert(logical_job_id, explanation.clone());
            if let Some(command) = selected {
                let resource_plan = self
                    .observe()?
                    .attempts
                    .iter()
                    .find(|a| a.command.attempt.command_id == command.attempt.command_id)
                    .expect("just prepared original Engine attempt")
                    .reserved_resource_units
                    .clone();
                let reservation_id = self.next_id("reservation");
                self.attempts.insert(
                    command.attempt.command_id.clone(),
                    AttemptLedger {
                        reservation_id,
                        resource_plan,
                        command: command.clone(),
                        reserved_at_host_ms: elapsed,
                        published_at_host_ms: elapsed,
                        dispatch: explanation,
                    },
                );
                let work_id = self.next_id("publish-command");
                self.pending_cloud_work
                    .push(CloudWorkItem::PublishCommand { work_id, command });
            }
        }
        Ok(())
    }
    pub fn tick(&mut self, now: Instant) -> Result<()> {
        if self.clock_anchor.is_none() || self.is_terminal() {
            return Ok(());
        }
        let elapsed = self.elapsed(now)?;
        if elapsed == self.observe()?.elapsed_ms {
            return Ok(());
        }
        self.advance_boundaries(elapsed, false)?;
        if self.is_terminal() {
            return Ok(());
        }
        let next = self.next_revision()?;
        self.engine
            .advance_external(uint(elapsed))
            .map_err(engine_error)?;
        self.revision = next;
        self.reconcile_lifecycle()?;
        self.refresh_probes(elapsed)?;
        self.withdraw_disabled_commands()?;
        self.schedule(elapsed)?;
        self.capture_frame()
    }
    fn message_replay(
        &self,
        receipt_id: &ObjectId,
        payload: &Value,
    ) -> Result<Option<ReceiptAdmission>> {
        if let Some(old) = self.messages.get(receipt_id) {
            if &old.payload != payload {
                return Err(fault(
                    ErrorCode::InvalidInput,
                    "Receipt identity has another payload",
                ));
            }
            let mut duplicate = old.latest.clone();
            duplicate.decision = ReceiptAdmissionDecision::Duplicate;
            duplicate.business_effect_committed = false;
            return Ok(Some(duplicate));
        }
        Ok(None)
    }
    fn admit_message(
        &mut self,
        receipt_id: ObjectId,
        payload: Value,
        production: Option<DeviceReceipt>,
        decision: ReceiptAdmissionDecision,
        effect: bool,
        reason: Option<String>,
        elapsed: u64,
    ) -> ReceiptAdmission {
        self.admission_sequence += 1;
        let received = self
            .messages
            .get(&receipt_id)
            .map_or(elapsed, |m| m.first_received_elapsed_ms);
        let admission = ReceiptAdmission {
            receipt_id: receipt_id.clone(),
            decision,
            execution_revision: uint(self.revision),
            host_admission_sequence: uint(self.admission_sequence),
            host_received_elapsed_ms: uint(received),
            business_effect_committed: effect,
            reason: reason.map(|s| DeviceText::try_from(s).unwrap()),
        };
        self.admissions.push(admission.clone());
        self.messages.insert(
            receipt_id,
            AdmittedMessage {
                payload,
                production_receipt: production,
                latest: admission.clone(),
                first_received_elapsed_ms: received,
            },
        );
        admission
    }
    fn check_device_message(
        &self,
        session: &ExecutionSessionId,
        epoch: &ExecutionEpochId,
        device: &ObjectId,
        boot: &DeviceBootId,
        sequence: u64,
    ) -> Result<()> {
        if session != &self.identity.session_id
            || epoch != &self.identity.execution_epoch_id
            || self
                .devices
                .get(device)
                .is_none_or(|s| s.boot.as_ref() != Some(boot))
        {
            return Err(fault(
                ErrorCode::PermissionDenied,
                "Message is outside the original session/device/boot",
            ));
        }
        if self
            .device_sequences
            .contains_key(&(device.clone(), boot.clone(), sequence))
        {
            return Err(fault(
                ErrorCode::InvalidInput,
                "A boot sequence already belongs to another original cloud receipt",
            ));
        }
        Ok(())
    }
    pub fn apply_observation(
        &mut self,
        receipt_id: ObjectId,
        cloud_received_at: CloudTimestamp,
        observation: ExecutionTelemetryObservation,
        now: Instant,
    ) -> Result<ReceiptAdmission> {
        let payload = serde_json::json!({"kind":"telemetry","cloud_received_at":cloud_received_at,"observation":observation});
        if let Some(old) = self.message_replay(&receipt_id, &payload)? {
            return Ok(old);
        }
        self.check_device_message(
            &observation.session_id,
            &observation.execution_epoch_id,
            &observation.device_id,
            &observation.device_boot_id,
            observation.device_sequence.value(),
        )?;
        if self.clock_anchor.is_none() {
            return Err(fault(
                ErrorCode::InvalidInput,
                "Prepared observations cannot start production freshness",
            ));
        }
        if self.dataset_id.as_ref() != Some(&observation.dataset_id) {
            return Err(fault(
                ErrorCode::PermissionDenied,
                "Telemetry belongs to another dataset",
            ));
        }
        if self.is_terminal() {
            return Err(fault(
                ErrorCode::InvalidInput,
                "Execution observation window is settled; retain later raw telemetry in cloud",
            ));
        }
        let elapsed = self.elapsed(now)?;
        self.advance_boundaries(elapsed, false)?;
        if self.is_terminal() {
            return Err(fault(
                ErrorCode::InvalidInput,
                "Execution observation window ended before this telemetry",
            ));
        }
        let probe = self.devices[&observation.device_id]
            .probe
            .clone()
            .filter(|p| {
                observation.observation_probe_id.as_ref() == Some(&p.probe_id)
                    && p.issued_at_host_ms.value() <= elapsed
                    && elapsed <= p.expires_at_host_ms.value()
            });
        let next = self.next_revision()?;
        self.engine
            .advance_external(uint(elapsed))
            .map_err(engine_error)?;
        self.revision = next;
        let admission = self.admit_message(
            receipt_id.clone(),
            payload,
            None,
            if probe.is_some() {
                ReceiptAdmissionDecision::Applied
            } else {
                ReceiptAdmissionDecision::Rejected
            },
            false,
            probe
                .is_none()
                .then(|| "Original post-run probe is missing or expired".to_owned()),
            elapsed,
        );
        self.device_sequences.insert(
            (
                observation.device_id.clone(),
                observation.device_boot_id.clone(),
                observation.device_sequence.value(),
            ),
            receipt_id.clone(),
        );
        if let Some(probe) = probe {
            let slot = self.devices.get_mut(&observation.device_id).unwrap();
            slot.latest = Some(HostObservedDevice {
                receipt_id,
                host_admission_sequence: admission.host_admission_sequence.clone(),
                host_received_elapsed_ms: uint(elapsed),
                observation,
                observation_probe: probe,
            });
            slot.probe = None;
        }
        self.reconcile_lifecycle()?;
        self.refresh_probes(elapsed)?;
        self.schedule(elapsed)?;
        self.capture_frame()?;
        Ok(admission)
    }
    pub fn apply_receipt(
        &mut self,
        receipt_id: ObjectId,
        cloud_received_at: CloudTimestamp,
        receipt: DeviceReceipt,
        now: Instant,
    ) -> Result<ReceiptAdmission> {
        let payload = serde_json::json!({"kind":"receipt","cloud_received_at":cloud_received_at,"observation":receipt});
        if let Some(old) = self.message_replay(&receipt_id, &payload)? {
            return Ok(old);
        }
        self.check_device_message(
            &receipt.session_id,
            &receipt.execution_epoch_id,
            &receipt.device_id,
            &receipt.device_boot_id,
            receipt.device_sequence.value(),
        )?;
        if self.clock_anchor.is_none() {
            return Err(fault(
                ErrorCode::InvalidInput,
                "Prepared session cannot accept production receipts",
            ));
        }
        if self.is_terminal() {
            return Err(fault(
                ErrorCode::InvalidInput,
                "Settled execution cannot admit a new production receipt",
            ));
        }
        let elapsed = self.elapsed(now)?;
        self.advance_boundaries(elapsed, true)?;
        if self.is_terminal() {
            return Err(fault(
                ErrorCode::InvalidInput,
                "Execution settled before this new production receipt",
            ));
        }
        let next = self.next_revision()?;
        let outcome = self
            .engine
            .apply_external_receipt(receipt.clone(), uint(elapsed));
        // A rejected message still has its real Host receive point. It cannot
        // place an admission beyond an older Engine observation frontier.
        if self.observe()?.elapsed_ms < elapsed {
            self.engine
                .advance_external(uint(elapsed))
                .map_err(engine_error)?;
        }
        self.revision = next;
        let (decision, effect, reason, committed) = match outcome {
            Ok(result) => {
                let pending = matches!(result.status, ExternalReceiptStatus::RetainedUnapplied);
                (
                    if result.duplicate {
                        ReceiptAdmissionDecision::Duplicate
                    } else if pending {
                        ReceiptAdmissionDecision::PendingDependency
                    } else {
                        ReceiptAdmissionDecision::Applied
                    },
                    !result.duplicate
                        && matches!(
                            result.status,
                            ExternalReceiptStatus::Started
                                | ExternalReceiptStatus::Completed
                                | ExternalReceiptStatus::Failed
                        ),
                    pending.then(|| "Original Started receipt is still required".to_owned()),
                    (!pending).then(|| uint(result.committed_at_ms)),
                )
            }
            Err(e) if e.code == "external.material_changed" => (
                ReceiptAdmissionDecision::PendingDependency,
                false,
                Some(e.to_string()),
                None,
            ),
            Err(e) => (
                ReceiptAdmissionDecision::Rejected,
                false,
                Some(e.to_string()),
                None,
            ),
        };
        let admission = self.admit_message(
            receipt_id.clone(),
            payload,
            Some(receipt.clone()),
            decision,
            effect,
            reason,
            elapsed,
        );
        self.receipt_evidence.push(HostReceiptEvidence {
            receipt: receipt.clone(),
            admission: admission.clone(),
            committed_at_host_ms: committed,
        });
        self.device_sequences.insert(
            (
                receipt.device_id.clone(),
                receipt.device_boot_id.clone(),
                receipt.device_sequence.value(),
            ),
            receipt_id,
        );
        self.reconcile_lifecycle()?;
        self.schedule(elapsed)?;
        self.capture_frame()?;
        // Each later dependency resolution is its own immutable revision, even
        // when it shares a millisecond with the newly admitted Started.
        let pending: Vec<_> = self
            .messages
            .iter()
            .filter_map(|(id, m)| {
                (matches!(admission.decision, ReceiptAdmissionDecision::Applied)
                    && matches!(receipt.receipt, DeviceReceiptKind::Started {})
                    && matches!(
                        m.latest.decision,
                        ReceiptAdmissionDecision::PendingDependency
                    )
                    && m.production_receipt
                        .as_ref()
                        .is_some_and(|r| r.command_id == receipt.command_id))
                .then_some((id.clone(), m.clone()))
            })
            .collect();
        for (id, m) in pending {
            let original = m.production_receipt.clone().unwrap();
            let next = self.next_revision()?;
            match self
                .engine
                .apply_external_receipt(original.clone(), uint(elapsed))
            {
                Ok(result)
                    if !matches!(result.status, ExternalReceiptStatus::RetainedUnapplied) =>
                {
                    self.revision = next;
                    let resolved = self.admit_message(
                        id,
                        m.payload,
                        Some(original.clone()),
                        ReceiptAdmissionDecision::Applied,
                        matches!(
                            result.status,
                            ExternalReceiptStatus::Started
                                | ExternalReceiptStatus::Completed
                                | ExternalReceiptStatus::Failed
                        ),
                        None,
                        elapsed,
                    );
                    self.receipt_evidence.push(HostReceiptEvidence {
                        receipt: original,
                        admission: resolved,
                        committed_at_host_ms: Some(uint(result.committed_at_ms)),
                    });
                    self.reconcile_lifecycle()?;
                    self.schedule(elapsed)?;
                    self.capture_frame()?;
                }
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.code,
                        "external.material_changed" | "external.sequence" | "external.state"
                    ) => {}
                Err(e) => return Err(engine_error(e)),
            }
        }
        Ok(admission)
    }
    pub fn cloud_work(&self) -> CloudWorkBatch {
        CloudWorkBatch {
            identity: self.identity.clone(),
            items: self.pending_cloud_work.clone(),
        }
    }
    pub fn acknowledge_cloud_work(&mut self, ack: CloudEffectAck, now: Instant) -> Result<()> {
        let id = ack.work_id().clone();
        let value = serde_json::to_value(&ack).unwrap();
        if let Some(old) = self.acknowledged_work.get(&id) {
            let canonical = |v: &Value| {
                let mut v = v.clone();
                match v["kind"].as_str() {
                    Some("command_stored") => {
                        v.as_object_mut().unwrap().remove("storage_status");
                    }
                    Some("snapshot_published") => {
                        v["ack"]
                            .as_object_mut()
                            .unwrap()
                            .remove("publication_status");
                    }
                    _ => {}
                }
                v
            };
            let already_settled = value["kind"] == "withdrawal_result"
                && value["state"] == "already_settled"
                && old["kind"] == value["kind"]
                && old["command_id"] == value["command_id"]
                && old["work_id"] == value["work_id"];
            return if canonical(old) == canonical(&value) || already_settled {
                Ok(())
            } else {
                Err(fault(
                    ErrorCode::InvalidInput,
                    "Cloud work acknowledgement payload changed",
                ))
            };
        }
        let index = self
            .pending_cloud_work
            .iter()
            .position(|w| w.work_id() == &id)
            .ok_or_else(|| {
                fault(
                    ErrorCode::MissingReference,
                    "Cloud work identity is not registered",
                )
            })?;
        let original = self.pending_cloud_work[index].clone();
        match (&original, &ack) {
            (
                CloudWorkItem::PublishCommand { command, .. },
                CloudEffectAck::CommandStored { command_id, .. },
            ) if command_id == &command.attempt.command_id => {}
            (
                CloudWorkItem::PublishSnapshot { publication, .. },
                CloudEffectAck::SnapshotPublished { ack, .. },
            ) if ack.session_id == self.identity.session_id
                && ack.execution_epoch_id == self.identity.execution_epoch_id
                && ack.execution_revision.value()
                    == publication.snapshot.execution_revision.value()
                && ack.observation_frontier.value()
                    == publication.snapshot.observation_frontier.value() =>
            {
                self.cloud_published_revision = Some(
                    self.cloud_published_revision
                        .unwrap_or(0)
                        .max(ack.execution_revision.value()),
                );
            }
            (
                CloudWorkItem::WithdrawCommand { command_id, .. },
                CloudEffectAck::WithdrawalResult {
                    command_id: confirmed,
                    state,
                    ..
                },
            ) if confirmed == command_id => {
                if matches!(
                    state,
                    CloudResponseVariant0ResultWithdrawalDataState::WithdrawnBeforeDelivery
                ) {
                    let elapsed = self.elapsed(now)?;
                    self.advance_boundaries(elapsed, false)?;
                    let next = self.next_revision()?;
                    self.engine
                        .confirm_external_withdrawal(command_id, uint(elapsed))
                        .map_err(engine_error)?;
                    self.revision = next;
                    self.reconcile_lifecycle()?;
                    self.schedule(elapsed)?;
                    self.capture_frame()?;
                }
            }
            _ => {
                return Err(fault(
                    ErrorCode::InvalidInput,
                    "Cloud acknowledgement does not match the original work",
                ));
            }
        }
        self.pending_cloud_work.remove(index);
        self.acknowledged_work.insert(id, value);
        Ok(())
    }
    pub fn queue_publication(&mut self, publication: SnapshotPublication) -> Result<()> {
        if !equal(&publication.snapshot.identity, &self.identity)
            || publication.snapshot.execution_revision.value() != self.revision
        {
            return Err(fault(
                ErrorCode::InvalidInput,
                "Publication must be the original current execution projection",
            ));
        }
        if let Some(old) = self.publications.get(&self.revision) {
            return if equal(old, &publication) {
                Ok(())
            } else {
                Err(fault(
                    ErrorCode::InvalidInput,
                    "A revision already has another publication",
                ))
            };
        }
        self.publications.insert(self.revision, publication.clone());
        let work_id = self.next_id("publish-snapshot");
        self.pending_cloud_work
            .push(CloudWorkItem::PublishSnapshot {
                work_id,
                publication,
            });
        Ok(())
    }
}
