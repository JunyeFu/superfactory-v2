//! Additive internal Rust execution handoff. Wire ownership stays in sf-domain.
use super::*;
use sf_domain::cloud_execution as wire;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalError {
    pub code: &'static str,
    pub message: String,
}
impl std::fmt::Display for ExternalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ExternalError {}
fn error(code: &'static str, message: impl Into<String>) -> ExternalError {
    ExternalError {
        code,
        message: message.into(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalAttemptState {
    Reserved,
    Published,
    Accepted,
    Started,
    Completed,
    CancelledNotStarted,
    WithdrawnUnpublished,
    ReconciliationRequired,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalReceiptStatus {
    Accepted,
    Started,
    Completed,
    CancelledNotStarted,
    RetainedUnapplied,
    Failed,
}
#[derive(Debug, Clone)]
pub struct ExternalReceiptOutcome {
    pub status: ExternalReceiptStatus,
    pub duplicate: bool,
    pub committed_at_ms: u64,
    pub command_id: DispatchCommandId,
    pub logical_job_id: ObjectId,
    pub device_reported_duration_ms: Option<u64>,
}
#[derive(Debug, Clone)]
pub struct ExternalAttemptView {
    pub command: wire::Command,
    pub state: ExternalAttemptState,
    pub pending_items: Vec<(ObjectId, u64)>,
    pub pending_input_ordinals: Vec<SafeUint>,
    pub reserved_resource_units: Vec<wire::ResourceUnits>,
    pub unapplied_receipts: Vec<wire::DeviceReceipt>,
}
#[derive(Debug, Clone)]
pub struct ExternalLogicalWork {
    pub logical_job_id: ObjectId,
    pub production_task_id: ObjectId,
    pub route_step_id: ObjectId,
    pub cycle: u64,
}
/// Actual same-Engine prefix; not a prediction RunResult or a new wire DTO.
#[derive(Debug, Clone)]
pub struct ExternalObservation {
    pub identity: wire::ExecutionIdentity,
    pub clock_started: bool,
    pub elapsed_ms: u64,
    pub dispatch_stopped: bool,
    pub drained: bool,
    pub goal_met: bool,
    pub queued_logical_jobs: Vec<ObjectId>,
    pub logical_work: Vec<ExternalLogicalWork>,
    pub attempts: Vec<ExternalAttemptView>,
    pub items: Vec<RuntimeItem>,
    pub jobs: Vec<JobState>,
    pub events: Vec<SimulationEvent>,
    pub resource_claims: Vec<ResourceClaim>,
    pub phase_work_intervals: Vec<PhaseWorkInterval>,
    pub acceptance_events: Vec<AcceptanceEvent>,
    pub allocations: Vec<MaterialAllocation>,
    pub order_outcomes: Vec<OrderOutcome>,
}
struct FrozenBinding {
    binding: wire::CandidateBinding,
    jobs: Vec<Job>,
}
struct Attempt {
    command: wire::Command,
    index: usize,
    spec: Job,
    resources: BTreeMap<ObjectId, u64>,
    state: ExternalAttemptState,
    started: bool,
    admitted: Vec<(wire::DeviceReceipt, ExternalReceiptOutcome)>,
    unapplied: Vec<wire::DeviceReceipt>,
}
impl Attempt {
    fn reserves(&self) -> bool {
        !self.started
            && matches!(
                self.state,
                ExternalAttemptState::Reserved
                    | ExternalAttemptState::Published
                    | ExternalAttemptState::Accepted
                    | ExternalAttemptState::ReconciliationRequired
            )
    }
}
pub(crate) struct ExternalExecution {
    identity: wire::ExecutionIdentity,
    policy: wire::ExecutionPolicy,
    candidates: BTreeMap<ObjectId, FrozenBinding>,
    attempts: BTreeMap<DispatchCommandId, Attempt>,
    live: BTreeMap<usize, DispatchCommandId>,
    ordinals: BTreeMap<usize, u64>,
    activated: bool,
    pub(crate) stop_dispatch: bool,
    cutoff: Option<Box<ExternalObservation>>,
}
impl ExternalExecution {
    pub(crate) fn reserved_units(&self, resource: &ObjectId) -> u64 {
        self.attempts
            .values()
            .filter(|a| a.reserves())
            .map(|a| a.resources.get(resource).copied().unwrap_or(0))
            .sum()
    }
}
fn same_receipt(a: &wire::DeviceReceipt, b: &wire::DeviceReceipt) -> bool {
    if a.device_sequence != b.device_sequence
        || a.device_timestamp.as_str() != b.device_timestamp.as_str()
    {
        return false;
    }
    match (&a.receipt, &b.receipt) {
        (wire::DeviceReceiptKind::Accepted {}, wire::DeviceReceiptKind::Accepted {})
        | (wire::DeviceReceiptKind::Started {}, wire::DeviceReceiptKind::Started {})
        | (
            wire::DeviceReceiptKind::CancelledNotStarted {},
            wire::DeviceReceiptKind::CancelledNotStarted {},
        )
        | (
            wire::DeviceReceiptKind::CancelRejectedStarted {},
            wire::DeviceReceiptKind::CancelRejectedStarted {},
        ) => true,
        (
            wire::DeviceReceiptKind::Completed {
                device_reported_duration_ms: a,
            },
            wire::DeviceReceiptKind::Completed {
                device_reported_duration_ms: b,
            },
        ) => a.as_ref().map(|x| x.value()) == b.as_ref().map(|x| x.value()),
        (
            wire::DeviceReceiptKind::Failed {
                code: a,
                message: x,
            },
            wire::DeviceReceiptKind::Failed {
                code: b,
                message: y,
            },
        ) => a.as_str() == b.as_str() && x.as_str() == y.as_str(),
        _ => false,
    }
}

impl Engine {
    /// Freeze one operation and its legal alternative devices. Compilation of a
    /// candidate is pure; only the original logical jobs enter this Engine.
    pub fn new_external(
        input: EngineInput,
        identity: wire::ExecutionIdentity,
        policy: wire::ExecutionPolicy,
        candidates: Vec<wire::CandidateBinding>,
    ) -> Result<Self, ExternalError> {
        if identity.project_id != input.project.id
            || input.scenario.window.start_ms.0.value() != 0
            || policy.source_prediction_window.end_ms.value()
                != input.scenario.window.end_ms.0.value()
            || policy.report_cutoff_ms.value() == 0
            || amount(&policy.target_quantity) == 0
            || policy.target_quantity.value.decimal_places.value() != 0
        {
            return Err(error(
                "external.policy",
                "Exact Project, zero-based prediction window and positive discrete target/cutoff required",
            ));
        }
        let mut engine =
            Self::new(input).map_err(|issues| error("external.input", format!("{issues:?}")))?;
        if engine.jobs.is_empty() || candidates.is_empty() {
            return Err(error(
                "external.scope",
                "Actual logical work and frozen candidates required",
            ));
        }
        let operation = engine.jobs[0].spec.operation_ref.clone();
        for job in &engine.jobs {
            let s = &job.spec;
            if !same_module(&s.operation_ref, &operation)
                || s.phases.len() != 1
                || s.phases[0].pause_resume
                || s.transport.is_some()
                || s.setup.is_some()
                || s.family.is_some()
                || s.preparation.is_some()
                || s.quality.is_some()
                || s.op().material_flow.is_some()
                || !s.predecessors.is_empty()
                || s.outputs.len() != 1
                || s.outputs[0].amount != 1
                || s.outputs[0].unit != policy.target_quantity.unit
            {
                return Err(error(
                    "external.unsupported",
                    "This handoff accepts one independent single-phase unit-producing operation; transport/setup/quality/flow/multiphase routes require their own external binding",
                ));
            }
        }
        if engine.jobs.len() as u64 != amount(&policy.target_quantity) {
            return Err(error(
                "external.target",
                "Frozen unit work count must equal the explicit target",
            ));
        }
        let mut frozen = BTreeMap::new();
        for binding in candidates {
            if frozen.contains_key(&binding.binding_id)
                || !same_module(&binding.operation_ref, &operation)
            {
                return Err(error(
                    "external.binding",
                    "Duplicate binding or different operation",
                ));
            }
            let device = engine
                .input
                .project
                .devices
                .iter()
                .find(|d| d.id == binding.device_id)
                .ok_or_else(|| error("external.binding", "Candidate device missing"))?;
            if !same_module(&device.module_ref, &binding.device_module_ref) {
                return Err(error(
                    "external.binding",
                    "Candidate device exact version differs",
                ));
            }
            let mut candidate_input = engine.input.clone();
            for route in &mut candidate_input.project.routes {
                for step in &mut route.steps {
                    if same_module(&step.operation_ref, &operation) {
                        step.device_id = binding.device_id.clone();
                        step.resource_bindings = binding
                            .resources
                            .iter()
                            .map(|r| (r.role_id.clone(), r.resource_id.clone()))
                            .collect();
                    }
                }
            }
            let (compiled, _) = crate::compile::compile(&candidate_input).map_err(|issues| {
                error(
                    "external.binding",
                    format!("Candidate compilation rejected: {issues:?}"),
                )
            })?;
            if compiled.len() != engine.jobs.len() {
                return Err(error(
                    "external.binding",
                    "Candidate changed logical work count",
                ));
            }
            let mut specs = Vec::new();
            for base in &engine.jobs {
                let spec = compiled
                    .iter()
                    .find(|j| j.id == base.spec.id)
                    .ok_or_else(|| {
                        error(
                            "external.binding",
                            "Candidate changed logical task/cycle/step identity",
                        )
                    })?
                    .clone();
                if spec.phases.len() != 1
                    || spec.phases[0].id != binding.phase_id
                    || spec.phases[0].work != binding.nominal_work_ms.value()
                    || spec.phases[0].pause_resume
                    || spec.family.is_some()
                    || spec.op().material_flow.is_some()
                    || spec.transport.is_some()
                    || spec.setup.is_some()
                    || spec.preparation.is_some()
                    || spec.quality.is_some()
                    || !spec.predecessors.is_empty()
                {
                    return Err(error(
                        "external.binding",
                        "Candidate phase/model/nominal duration differs from compiler",
                    ));
                }
                match &spec.execution {
                    JobExecutionRef::ProjectDevice {
                        device_id,
                        execution_capacity_ref,
                        ..
                    } if *device_id == binding.device_id
                        && execution_capacity_ref.id == binding.execution_capacity_ref.id
                        && matches!(binding.execution_capacity_ref.kind, ObjectKind::Resource) => {}
                    _ => {
                        return Err(error(
                            "external.binding",
                            "Execution device/capacity mapping differs",
                        ));
                    }
                }
                for (declared, actual) in [
                    (&binding.input_ports, &spec.inputs),
                    (&binding.output_ports, &spec.outputs),
                ] {
                    let mut roles = BTreeSet::new();
                    if declared.len() != actual.len()
                        || declared.iter().any(|p| {
                            !roles.insert(p.role_id.clone())
                                || !actual.iter().any(|m| {
                                    m.role == p.role_id
                                        && m.port == p.port_id
                                        && m.location == p.location_id
                                        && m.item_type.id == p.item_type_id
                                        && m.unit == p.unit_id
                                })
                        })
                    {
                        return Err(error(
                            "external.binding",
                            "Exact role/port/location/item type/unit mapping differs",
                        ));
                    }
                }
                let requirements: BTreeMap<_, _> = spec
                    .op()
                    .resource_roles
                    .iter()
                    .chain(spec.op().phases.iter().flat_map(|p| &p.phase_resources))
                    .map(|r| (r.role.clone(), amount(&r.quantity)))
                    .collect();
                let mut roles = BTreeSet::new();
                if binding.resources.len() != requirements.len()
                    || binding.resources.iter().any(|r| {
                        !roles.insert(r.role_id.clone())
                            || requirements.get(&r.role_id) != Some(&r.units.value())
                            || spec
                                .job_resources
                                .get(&r.resource_id)
                                .copied()
                                .unwrap_or(0)
                                .max(
                                    spec.phases[0]
                                        .resources
                                        .get(&r.resource_id)
                                        .copied()
                                        .unwrap_or(0),
                                )
                                < r.units.value()
                    })
                {
                    return Err(error(
                        "external.binding",
                        "Resource role/units differ from frozen compiled requirements",
                    ));
                }
                specs.push(spec);
            }
            frozen.insert(
                binding.binding_id.clone(),
                FrozenBinding {
                    binding,
                    jobs: specs,
                },
            );
        }
        engine.external = Some(ExternalExecution {
            identity,
            policy,
            candidates: frozen,
            attempts: BTreeMap::new(),
            live: BTreeMap::new(),
            ordinals: BTreeMap::new(),
            activated: false,
            stop_dispatch: false,
            cutoff: None,
        });
        Ok(engine)
    }

    pub fn activate_external(&mut self) -> Result<(), ExternalError> {
        let e = self
            .external
            .as_mut()
            .ok_or_else(|| error("external.mode", "Not an external Engine"))?;
        if e.activated {
            return Err(error(
                "external.activation",
                "Clock epoch already activated",
            ));
        }
        e.activated = true;
        self.status = ComputationStatus::Running;
        self.external_advance_to(0);
        Ok(())
    }
    pub fn advance_external(&mut self, elapsed: SafeUint) -> Result<(), ExternalError> {
        self.external_observe_time(elapsed, false)
    }
    fn external_observe_time(
        &mut self,
        elapsed: SafeUint,
        receipt_at_cutoff: bool,
    ) -> Result<(), ExternalError> {
        self.external_check_time(elapsed.value())?;
        if matches!(self.status, ComputationStatus::Completed) {
            return Ok(());
        }
        let cutoff = self
            .external
            .as_ref()
            .unwrap()
            .policy
            .report_cutoff_ms
            .value();
        if self.external.as_ref().unwrap().cutoff.is_none() && cutoff <= elapsed.value() {
            self.external_advance_to(cutoff);
            self.external_stop_dispatch();
            if !receipt_at_cutoff || elapsed.value() > cutoff {
                self.external_freeze_cutoff()?;
            }
        }
        if !matches!(self.status, ComputationStatus::Completed) {
            self.external_advance_to(elapsed.value());
        }
        Ok(())
    }
    pub(super) fn external_stop_dispatch(&mut self) {
        self.external.as_mut().unwrap().stop_dispatch = true;
        let unpublished: Vec<_> = self
            .external
            .as_ref()
            .unwrap()
            .attempts
            .iter()
            .filter(|(_, a)| a.state == ExternalAttemptState::Reserved)
            .map(|(id, _)| id.clone())
            .collect();
        for id in unpublished {
            self.external_release_unpublished(&id);
        }
        self.external_settle();
    }
    fn external_freeze_cutoff(&mut self) -> Result<(), ExternalError> {
        let e = self.external.as_ref().unwrap();
        if e.cutoff.is_none() && self.now == e.policy.report_cutoff_ms.value() {
            let view = self.external_observation()?;
            self.external.as_mut().unwrap().cutoff = Some(Box::new(view));
        }
        Ok(())
    }
    fn external_check_time(&self, elapsed: u64) -> Result<(), ExternalError> {
        let e = self
            .external
            .as_ref()
            .ok_or_else(|| error("external.mode", "Not an external Engine"))?;
        if !e.activated {
            return Err(error(
                "external.prepared",
                "Activate the original Host monotonic epoch before dispatch or receipt admission",
            ));
        }
        if elapsed < self.now {
            return Err(error("external.clock", "Host elapsed cannot move backward"));
        }
        Ok(())
    }
    fn external_advance_to(&mut self, elapsed: u64) {
        while let Some(next) = self
            .queue
            .peek()
            .map(|e| e.0.at)
            .filter(|at| *at <= elapsed)
        {
            self.now = next;
            while self.queue.peek().is_some_and(|e| e.0.at == self.now) {
                let event = self.queue.pop().unwrap().0;
                assert!(
                    !matches!(event.kind, Boundary::Work(..)),
                    "external phase must never schedule nominal completion"
                );
                self.boundary(event.kind);
            }
            self.external_settle();
        }
        self.now = elapsed;
        self.external_settle();
    }
    fn external_settle(&mut self) {
        loop {
            if self.try_arrival_one() || self.transfer_one() {
                continue;
            }
            self.accept_orders();
            break;
        }
        self.computed = Some(self.now);
        self.record_job_history();
        let e = self.external.as_ref().unwrap();
        let unresolved = e
            .attempts
            .values()
            .any(|a| a.reserves() || a.started && a.state != ExternalAttemptState::Completed);
        let output_wait = self.jobs.iter().any(|j| {
            j.state.started_at_ms.is_some() && !matches!(j.state.state, JobStatus::Completed)
        });
        if !unresolved
            && !output_wait
            && (e.stop_dispatch
                || self
                    .jobs
                    .iter()
                    .all(|j| matches!(j.state.state, JobStatus::Completed)))
        {
            self.status = ComputationStatus::Completed;
        }
    }

    /// Before any cloud/network publication, cross the publication barrier.
    /// A returned Command is not permission to transmit before mark_published.
    pub fn prepare_external(
        &mut self,
        attempt: wire::AttemptRef,
        boot: DeviceBootId,
        elapsed: SafeUint,
    ) -> Result<wire::Command, ExternalError> {
        self.advance_external(elapsed)?;
        let e = self.external.as_ref().unwrap();
        if e.stop_dispatch || !matches!(self.status, ComputationStatus::Running) {
            return Err(error("external.cutoff", "New dispatch is stopped"));
        }
        if e.attempts.contains_key(&attempt.command_id) {
            return Err(error(
                "external.command",
                "Command ID has already been used",
            ));
        }
        let index = self
            .jobs
            .iter()
            .position(|j| j.spec.id == attempt.logical_job_id)
            .ok_or_else(|| {
                error(
                    "external.logical_job",
                    "Unknown original logical task/cycle/step",
                )
            })?;
        if e.live.contains_key(&index) || self.jobs[index].state.started_at_ms.is_some() {
            return Err(error(
                "external.live_attempt",
                "Published, unknown, started or completed work cannot be reassigned",
            ));
        }
        if e.ordinals
            .get(&index)
            .is_some_and(|old| attempt.attempt_ordinal.value() <= *old)
        {
            return Err(error(
                "external.ordinal",
                "New attempt ordinal must increase",
            ));
        }
        let candidate = e
            .candidates
            .get(&attempt.selected_binding_id)
            .ok_or_else(|| error("external.binding", "Not a frozen legal candidate"))?;
        let spec = candidate.jobs[index].clone();
        let device_id = candidate.binding.device_id.clone();
        if attempt.phase_id != candidate.binding.phase_id || spec.release > self.now {
            return Err(error(
                "external.phase",
                "Wrong phase or logical work not yet released",
            ));
        }
        let old = std::mem::replace(&mut self.jobs[index].spec, spec.clone());
        let resource_plan = self.resource_plan(index);
        let resources = self.requirements(index);
        self.jobs[index].spec = old;
        resource_plan.map_err(|reasons| error("external.resources", format!("{reasons:?}")))?;
        self.stock
            .reserve_external(attempt.command_id.clone(), &spec, &self.input.project)
            .map_err(|reasons| error("external.material", format!("{reasons:?}")))?;
        let command = wire::Command {
            contract_version: wire::ExecutionContractVersion::Draft11,
            identity: self.external.as_ref().unwrap().identity.clone(),
            attempt: attempt.clone(),
            device_id,
            action: wire::DeviceAction::ProcessUnit,
            parameters: wire::CommandParameters {
                quantity: quantity(1, &spec.outputs[0].unit),
                phase_id: attempt.phase_id.clone(),
                nominal_work_ms: uint(spec.phases[0].work),
            },
            target_device_boot_id: boot,
        };
        let e = self.external.as_mut().unwrap();
        e.live.insert(index, attempt.command_id.clone());
        e.ordinals.insert(index, attempt.attempt_ordinal.value());
        e.attempts.insert(
            attempt.command_id.clone(),
            Attempt {
                command: command.clone(),
                index,
                spec,
                resources,
                state: ExternalAttemptState::Reserved,
                started: false,
                admitted: vec![],
                unapplied: vec![],
            },
        );
        Ok(command)
    }
    pub fn mark_external_published(
        &mut self,
        command: &DispatchCommandId,
        elapsed: SafeUint,
    ) -> Result<wire::Command, ExternalError> {
        self.advance_external(elapsed)?;
        let e = self.external.as_mut().unwrap();
        let a = e
            .attempts
            .get_mut(command)
            .ok_or_else(|| error("external.command", "Unknown command"))?;
        if a.state != ExternalAttemptState::Reserved {
            return Err(error(
                "external.publication",
                "Only an unpublished reservation may cross publication barrier",
            ));
        }
        a.state = ExternalAttemptState::Published;
        Ok(a.command.clone())
    }
    pub fn withdraw_external_unpublished(
        &mut self,
        command: &DispatchCommandId,
    ) -> Result<(), ExternalError> {
        let e = self
            .external
            .as_ref()
            .ok_or_else(|| error("external.mode", "Not an external Engine"))?;
        if e.attempts
            .get(command)
            .is_none_or(|a| a.state != ExternalAttemptState::Reserved)
        {
            return Err(error(
                "external.publication",
                "Published or unknown command requires positive not-started cancellation proof",
            ));
        }
        self.external_release_unpublished(command);
        Ok(())
    }
    fn external_release_unpublished(&mut self, command: &DispatchCommandId) {
        self.stock.release_external(command);
        let e = self.external.as_mut().unwrap();
        let a = e.attempts.get_mut(command).unwrap();
        a.state = ExternalAttemptState::WithdrawnUnpublished;
        e.live.remove(&a.index);
    }

    /// The trusted Host has the cloud transaction's positive proof that this
    /// published command was never delivered. This is not a device receipt.
    pub fn confirm_external_withdrawal(
        &mut self, command: &DispatchCommandId, elapsed: SafeUint,
    ) -> Result<(), ExternalError> {
        self.advance_external(elapsed)?;
        let a = self.external.as_ref().unwrap().attempts.get(command)
            .ok_or_else(|| error("external.command", "Unknown command"))?;
        if a.state == ExternalAttemptState::CancelledNotStarted && a.admitted.is_empty() {
            return Ok(());
        }
        if a.state != ExternalAttemptState::Published || a.started || !a.unapplied.is_empty() {
            return Err(error("external.withdrawal", "Withdrawal proof conflicts with observed execution"));
        }
        self.stock.release_external(command);
        let e = self.external.as_mut().unwrap();
        let a = e.attempts.get_mut(command).unwrap();
        a.state = ExternalAttemptState::CancelledNotStarted;
        e.live.remove(&a.index);
        self.external_settle();
        Ok(())
    }

    pub fn apply_external_receipt(
        &mut self,
        receipt: wire::DeviceReceipt,
        elapsed: SafeUint,
    ) -> Result<ExternalReceiptOutcome, ExternalError> {
        self.external_check_time(elapsed.value())?;
        let e = self.external.as_ref().unwrap();
        let a = e
            .attempts
            .get(&receipt.command_id)
            .ok_or_else(|| error("external.command", "Unknown command"))?;
        if receipt.session_id != e.identity.session_id
            || receipt.execution_epoch_id != e.identity.execution_epoch_id
            || receipt.device_id != a.command.device_id
            || receipt.device_boot_id != a.command.target_device_boot_id
            || receipt.phase_id != a.command.attempt.phase_id
            || receipt.phase_token != a.command.attempt.phase_token
        {
            return Err(error(
                "external.identity",
                "Receipt session/epoch/command/device/boot/phase/token mismatch",
            ));
        }
        for (old, outcome) in &a.admitted {
            if old.device_sequence == receipt.device_sequence {
                if !same_receipt(old, &receipt) {
                    return Err(error(
                        "external.sequence",
                        "Different payload for admitted device sequence",
                    ));
                }
                let mut duplicate = outcome.clone();
                duplicate.duplicate = true;
                return Ok(duplicate);
            }
        }
        if a.unapplied.iter().any(|old| {
            old.device_sequence == receipt.device_sequence && !same_receipt(old, &receipt)
        }) {
            return Err(error(
                "external.sequence",
                "Different payload for retained device sequence",
            ));
        }
        if a.admitted
            .last()
            .is_some_and(|(old, _)| old.device_sequence.value() >= receipt.device_sequence.value())
        {
            return Err(error("external.sequence", "Old unadmitted device sequence"));
        }
        if matches!(
            a.state,
            ExternalAttemptState::Reserved
                | ExternalAttemptState::WithdrawnUnpublished
                | ExternalAttemptState::CancelledNotStarted
                | ExternalAttemptState::Completed
                | ExternalAttemptState::Failed
        ) {
            return Err(error(
                "external.state",
                "Receipt is incompatible with the frozen attempt state",
            ));
        }
        self.external_observe_time(elapsed, true)?;
        let mut a = self
            .external
            .as_mut()
            .unwrap()
            .attempts
            .remove(&receipt.command_id)
            .unwrap();
        let result = self.external_apply(&mut a, &receipt);
        self.external
            .as_mut()
            .unwrap()
            .attempts
            .insert(receipt.command_id.clone(), a);
        if result.is_ok() {
            self.external_settle();
        }
        self.external_freeze_cutoff()?;
        result
    }
    fn external_apply(
        &mut self,
        a: &mut Attempt,
        receipt: &wire::DeviceReceipt,
    ) -> Result<ExternalReceiptOutcome, ExternalError> {
        let mut outcome = ExternalReceiptOutcome {
            status: ExternalReceiptStatus::RetainedUnapplied,
            duplicate: false,
            committed_at_ms: self.now,
            command_id: receipt.command_id.clone(),
            logical_job_id: a.command.attempt.logical_job_id.clone(),
            device_reported_duration_ms: None,
        };
        match &receipt.receipt {
            wire::DeviceReceiptKind::Accepted {} if !a.started => {
                a.state = if a.unapplied.is_empty() {
                    ExternalAttemptState::Accepted
                } else {
                    ExternalAttemptState::ReconciliationRequired
                };
                outcome.status = ExternalReceiptStatus::Accepted;
            }
            wire::DeviceReceiptKind::Started {} if !a.started => {
                let inputs = match self.stock.take_external(
                    &receipt.command_id,
                    &a.spec,
                    &self.input.project,
                    self.now,
                ) {
                    Ok(inputs) => inputs,
                    Err(issue) => {
                        a.state = ExternalAttemptState::ReconciliationRequired;
                        if !a.unapplied.iter().any(|old| same_receipt(old, receipt)) {
                            a.unapplied.push(receipt.clone());
                        }
                        return Err(error("external.material_changed", issue.message));
                    }
                };
                let sources = inputs
                    .iter()
                    .map(|(id, _)| {
                        self.stock.items[id]
                            .split_from_ref
                            .clone()
                            .unwrap_or_else(|| id.clone())
                    })
                    .collect();
                self.inherit_splits(&sources);
                self.jobs[a.index].spec = a.spec.clone();
                self.jobs[a.index].state.execution_ref = a.spec.execution.clone();
                for (item, ordinal) in &inputs {
                    self.emit(
                        object(ObjectKind::Item, item),
                        SimulationEventPayload::ItemTransferred(TransferEvent {
                            item_ref: item.clone(),
                            from_holder: HolderRef::Location(
                                a.spec.inputs[*ordinal].location.clone(),
                            ),
                            to_holder: HolderRef::Job(a.spec.id.clone()),
                        }),
                    );
                }
                self.jobs[a.index].state.input_refs =
                    inputs.iter().map(|(id, _)| id.clone()).collect();
                self.jobs[a.index].inputs = inputs;
                self.acquire(a.index, a.spec.job_resources.clone(), false);
                self.jobs[a.index].state.started_at_ms = Some(time(self.now));
                self.emit(
                    object(ObjectKind::Job, &a.spec.id),
                    SimulationEventPayload::JobStarted(a.spec.id.clone()),
                );
                self.start_phase(a.index, MAX_SAFE_UINT);
                a.started = true;
                a.state = ExternalAttemptState::Started;
                outcome.status = ExternalReceiptStatus::Started;
            }
            wire::DeviceReceiptKind::Completed {
                device_reported_duration_ms,
            } if a.started => {
                let active = self.jobs[a.index]
                    .active
                    .as_mut()
                    .ok_or_else(|| error("external.phase", "No active original phase"))?;
                active.complete = true;
                active.stop = self.now;
                let token = active.token;
                self.boundary(Boundary::Work(a.index, 0, token));
                a.state = ExternalAttemptState::Completed;
                outcome.status = ExternalReceiptStatus::Completed;
                outcome.device_reported_duration_ms =
                    device_reported_duration_ms.as_ref().map(|n| n.value());
            }
            wire::DeviceReceiptKind::CancelledNotStarted {}
                if !a.started && a.unapplied.is_empty() =>
            {
                self.stock.release_external(&receipt.command_id);
                a.state = ExternalAttemptState::CancelledNotStarted;
                self.external.as_mut().unwrap().live.remove(&a.index);
                outcome.status = ExternalReceiptStatus::CancelledNotStarted;
            }
            wire::DeviceReceiptKind::Completed { .. }
            | wire::DeviceReceiptKind::CancelRejectedStarted {}
                if !a.started =>
            {
                outcome.duplicate = a.unapplied.iter().any(|old| same_receipt(old, receipt));
                if !outcome.duplicate {
                    a.unapplied.push(receipt.clone());
                }
                a.state = ExternalAttemptState::ReconciliationRequired;
                return Ok(outcome); // no Stock.take, no synthetic start, no output
            }
            wire::DeviceReceiptKind::CancelRejectedStarted {} if a.started => {
                outcome.status = ExternalReceiptStatus::Started;
            }
            wire::DeviceReceiptKind::Failed { .. } => {
                if a.started {
                    if let Some(active) = self.jobs[a.index].active.take() {
                        let delta = self.now - active.start;
                        if delta > 0 {
                            self.work.push(PhaseWorkInterval {
                                job_id: a.spec.id.clone(),
                                phase_id: a.spec.phases[0].id.clone(),
                                interval: IntervalMs {
                                    start_ms: time(active.start),
                                    end_ms: time(self.now),
                                },
                                effective_progress_start_ms: duration(active.progress),
                                effective_progress_end_ms: duration(active.progress + delta),
                                end_cause: PhaseWorkEndCause::Interrupted,
                            });
                        }
                        self.jobs[a.index].progress = active.progress + delta;
                    }
                    self.jobs[a.index].state.state = JobStatus::Paused;
                    a.state = ExternalAttemptState::Failed;
                    outcome.status = ExternalReceiptStatus::Failed;
                } else {
                    a.state = ExternalAttemptState::ReconciliationRequired;
                    a.unapplied.push(receipt.clone());
                    return Ok(outcome);
                }
            }
            _ => {
                return Err(error(
                    "external.state",
                    "Repeated/conflicting start, acceptance or cancellation cannot change original business ownership",
                ));
            }
        }
        a.unapplied
            .retain(|old| old.device_sequence != receipt.device_sequence);
        a.admitted.push((receipt.clone(), outcome.clone()));
        Ok(outcome)
    }

    pub fn external_observation(&self) -> Result<ExternalObservation, ExternalError> {
        let e = self
            .external
            .as_ref()
            .ok_or_else(|| error("external.mode", "Not an external Engine"))?;
        // Existing projection reads the real same Engine/Stock; no replacement
        // Run or result is constructed to simulate an external completion.
        let actual = self.snapshot();
        let accepted: u64 = actual
            .acceptance_events
            .iter()
            .map(|a| amount(&a.quantity))
            .sum();
        Ok(ExternalObservation {
            identity: e.identity.clone(),
            clock_started: e.activated,
            elapsed_ms: self.now,
            dispatch_stopped: e.stop_dispatch,
            drained: matches!(self.status, ComputationStatus::Completed),
            goal_met: accepted >= amount(&e.policy.target_quantity),
            queued_logical_jobs: self
                .jobs
                .iter()
                .filter(|j| j.state.started_at_ms.is_none())
                .map(|j| j.spec.id.clone())
                .collect(),
            logical_work: self.jobs.iter().map(|job| {
                let JobExecutionRef::ProjectDevice { route_step_id: Some(step), .. } = &job.spec.execution else {
                    return Err(error("external.logical_work", "External work requires its original route-step identity"));
                };
                Ok(ExternalLogicalWork { logical_job_id: job.spec.id.clone(),
                    production_task_id: job.spec.task.clone(), route_step_id: step.clone(), cycle: job.spec.cycle })
            }).collect::<Result<Vec<_>, ExternalError>>()?,
            attempts: e
                .attempts
                .values()
                .map(|a| ExternalAttemptView {
                    command: a.command.clone(),
                    state: a.state,
                    pending_items: self
                        .stock
                        .external_picks(&a.command.attempt.command_id)
                        .iter()
                        .map(|p| (p.item.clone(), p.amount))
                        .collect(),
                    pending_input_ordinals: self.stock.external_picks(&a.command.attempt.command_id)
                        .iter().map(|pick| SafeUint::try_from(pick.material as u64).unwrap()).collect(),
                    reserved_resource_units: a.resources.iter().map(|(resource_id, units)| wire::ResourceUnits {
                        resource_id: resource_id.clone(), units: wire::PositiveSafeUint::try_from(*units).unwrap(),
                    }).collect(),
                    unapplied_receipts: a.unapplied.clone(),
                })
                .collect(),
            items: actual.final_items,
            jobs: actual.jobs,
            events: actual.events,
            resource_claims: actual.resource_claims,
            phase_work_intervals: actual.phase_work_intervals,
            acceptance_events: actual.acceptance_events,
            allocations: actual.allocations,
            order_outcomes: actual.orders,
        })
    }
    pub fn external_cutoff_observation(&self) -> Option<&ExternalObservation> {
        self.external.as_ref()?.cutoff.as_deref()
    }
    /// Next original model event, without advancing or interpreting nominal work.
    pub fn next_external_boundary_ms(&self) -> Result<Option<SafeUint>, ExternalError> {
        if self.external.is_none() { return Err(error("external.mode", "Not an external Engine")); }
        Ok(self.queue.peek().map(|event| SafeUint::try_from(event.0.at).unwrap()))
    }
}
