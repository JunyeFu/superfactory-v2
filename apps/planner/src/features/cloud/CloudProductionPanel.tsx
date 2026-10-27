import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import type { ProjectState } from '../../../../../packages/commands/generated/typescript/ProjectState';
import type { ScenarioInput } from '../../../../../packages/commands/generated/typescript/ScenarioInput';
import type { CapabilityCatalog } from '../../../../../packages/commands/generated/typescript/CapabilityCatalog';
import type { CloudProductionConfiguration } from '../../../../../packages/commands/generated/typescript/CloudProductionConfiguration';
import type { ExecutionSourceSummary } from '../../../../../packages/commands/generated/typescript/ExecutionSourceSummary';
import type { PredictionRunReference } from '../../../../../packages/commands/generated/typescript/PredictionRunReference';
import type { CommandReadEnvelope } from '../../../../../packages/commands/generated/typescript/CommandReadEnvelope';
import type { CommandReceiptsPage } from '../../../../../packages/commands/generated/typescript/CommandReceiptsPage';
import type { SnapshotSelection } from '../../../../../packages/commands/generated/typescript/SnapshotSelection';
import type { ApiRequest } from '../../../../../packages/commands/generated/typescript/ApiRequest';
import {
  EXECUTION_CONTRACT, CloudProductionError, canControl, checkUtcRange, cloudContextKey, cloudErrorCode, controlExecution,
  exportCloudData, exportDirectoryUnavailable, exportExecutionReport, exportQuery, formatQuantity,
  hasCloudOperation, identityKey, recoverRegisteredCloudTask, parseQuantity, prepareExecution, readCloud,
  readExecution, readLocalView, validateQueryPage,
} from '../../services/cloudProduction';
import type { CloudContext, CloudExportDirectories, CloudTaskRecord, ControlAction, DatasetDescriptor,
  DaaSQuery, DaaSQueryResult, CloudDataExportSelection, ExecutionIdentity, ExecutionReadEnvelope } from '../../services/cloudProduction';
import './cloudProduction.css';
import type { ExecutionObservationSelection } from '../../services/executionPresentation';

export type ExecutionViewSelection = ExecutionObservationSelection;
export interface CloudProductionPanelProps {
  project: ProjectState | null;
  scenario: ScenarioInput | null;
  capabilities: CapabilityCatalog;
  hostContextId: string;
  onClose: () => void;
  onLocateDevice: (deviceId: string) => void;
  onInspectExecution: (selection: ExecutionViewSelection) => void;
  /** Actual terminal Run references from Workbench, never manufactured by this panel. */
  predictionRuns?: readonly PredictionRunReference[];
  directories?: CloudExportDirectories;
  /** Root can return a retained request after unmount; this only resumes observation. */
  retainedTask?: CloudTaskRecord | null;
  onTaskRegistered?: (record: CloudTaskRecord) => void;
  onUnappliedInputChanged?: (dirty: boolean) => void;
  onRefreshProject?: () => void | Promise<void>;
}
type Tab = 'devices' | 'data' | 'execution';
type Lane = 'configuration' | 'datasets' | 'catalog' | 'source' | 'query' | 'command' | 'receipts' | 'mutation' | 'request';
interface Source { identity: ExecutionIdentity; origin: 'host' | 'cloud'; selection: SnapshotSelection }
interface QueryPage {
  query: DaaSQuery; result: DaaSQueryResult; pin: CloudDataExportSelection;
  lastPosition: number | null; number: number; seenRows: number;
}
const tabs: readonly [Tab, string][] = [['devices', '设备'], ['data', '数据'], ['execution', '执行']];
const labels: Record<string, string> = {
  prepared: '已准备，尚未启动', running: '运行中', dispatch_paused: '新派工已暂停', draining: '等待已准入工作收尾',
  needs_resolution: '待核对', completed: 'Host已收尾', stopped: '已停止', failed: '失败',
  fresh: '新鲜', stale: '已过期', unknown: '未知', satisfied: '满足', not_satisfied: '未满足', unobserved: '未观察',
};
const text = (v: unknown) => v === null || v === undefined ? '未知' : String(v);
const errorText = (e: unknown) => e instanceof Error ? e.message : String(e);
function Json({ value, label = '完整原始字段' }: { value: unknown; label?: string }) {
  return <details className="cloud-json"><summary>{label}</summary><pre>{JSON.stringify(value, null, 2)}</pre></details>;
}
function safeInteger(raw: string, label: string, positive = false) {
  if (!/^\d+$/.test(raw) || !Number.isSafeInteger(Number(raw)) || (positive && Number(raw) === 0)) throw new CloudProductionError('input', `${label}需为${positive ? '正' : '非负'}安全整数。`);
  return Number(raw);
}
function Notice({ children }: { children: ReactNode }) { return <p className="cloud-notice">{children}</p>; }

/** Central workspace only; Root retains canvas, Host and Native ownership. */
export function CloudProductionPanel(props: CloudProductionPanelProps) {
  const { project, scenario, capabilities, hostContextId } = props;
  const context: CloudContext | null = project ? { projectId: project.id, projectRevision: project.revision, hostContextId } : null;
  const key = `${context ? cloudContextKey(context) : 'none'}:${scenario?.id ?? ''}:${scenario?.revision ?? ''}`;
  const liveKey = useRef(key); liveKey.current = key;
  const propsRef = useRef(props); propsRef.current = props;
  const lanes = useRef(new Map<Lane, AbortController>());
  const mounted = useRef(true);
  const selectedSource = useRef<Source | null>(null);
  const sourceGeneration = useRef(0);
  const previousFormOwner = useRef('');
  const sourcePollingPaused = useRef(false);
  const viewTimeEdited = useRef(false);
  const [tab, setTab] = useState<Tab>('devices');
  const [config, setConfig] = useState<CloudProductionConfiguration | null>(null);
  const [datasets, setDatasets] = useState<DatasetDescriptor[]>([]);
  const [catalog, setCatalog] = useState<ExecutionSourceSummary[]>([]);
  const [envelope, setEnvelope] = useState<ExecutionReadEnvelope | null>(null);
  const [source, setSource] = useState<Source | null>(null);
  const [busy, setBusy] = useState<string[]>([]);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const [taskRecord, setTaskRecord] = useState<CloudTaskRecord | null>(null);
  const [target, setTarget] = useState(''); const [unit, setUnit] = useState('');
  const [cutoff, setCutoff] = useState(''); const [maxAge, setMaxAge] = useState('');
  const [bindings, setBindings] = useState<string[]>([]);
  const [dirty, setDirty] = useState(false);
  const [dataSet, setDataSet] = useState(''); const [deviceFilter, setDeviceFilter] = useState('');
  const [utcFrom, setUtcFrom] = useState(''); const [utcUntil, setUtcUntil] = useState('');
  const [fromMs, setFromMs] = useState('0'); const [untilMs, setUntilMs] = useState('');
  const [aggregation, setAggregation] = useState('rows'); const [page, setPage] = useState<QueryPage | null>(null);
  const [command, setCommand] = useState<CommandReadEnvelope | null>(null);
  const [receipts, setReceipts] = useState<CommandReceiptsPage | null>(null);
  const [receiptPages, setReceiptPages] = useState(0);
  const [viewTime, setViewTime] = useState(''); const [prediction, setPrediction] = useState('');
  const [revisionDraft, setRevisionDraft] = useState('');
  const id = useId();
  const available = (op: ApiRequest['operation']) => hasCloudOperation(capabilities, op);
  const currentLatest = source?.origin === 'host' && source.selection.kind === 'latest' && envelope?.access.ownership === 'current_host_owner';
  const snapshot = envelope?.snapshot ?? null;
  const mutationBusy = busy.includes('mutation');
  const unresolvedTask = !!taskRecord && !['completed', 'failed', 'cancelled', 'rejected'].includes(taskRecord.status);

  function begin(lane: Lane) {
    lanes.current.get(lane)?.abort();
    const controller = new AbortController(); lanes.current.set(lane, controller);
    const captured = key;
    const live = () => mounted.current && liveKey.current === captured && lanes.current.get(lane) === controller && !controller.signal.aborted;
    setBusy(old => old.includes(lane) ? old : [...old, lane]);
    return { controller, live, context, lane };
  }
  function finish(lease: ReturnType<typeof begin>) {
    if (lease.live()) { lanes.current.delete(lease.lane); setBusy(old => old.filter(v => v !== lease.lane)); }
  }
  function fail(lease: ReturnType<typeof begin>, e: unknown) {
    if (!lease.live()) return;
    if (lease.lane === 'source') sourcePollingPaused.current = true;
    const code = cloudErrorCode(e);
    if (code === 'PermissionDenied' || code === 'NoActiveProject') {
      setConfig(null); setDatasets([]); setCatalog([]); setEnvelope(null); setSource(null); setPage(null); setCommand(null); setReceipts(null);
      selectedSource.current = null; sourceGeneration.current += 1;
      // Permission rejection clears protected views rather than disguising it as an empty query.
      for (const [lane, c] of lanes.current) if (lane !== 'mutation') c.abort();
      setBusy(old => old.filter(lane => lane === 'mutation'));
    }
    setError(errorText(e));
  }
  function abortReads() {
    for (const [lane, controller] of lanes.current) if (lane !== 'mutation') { controller.abort(); lanes.current.delete(lane); }
    setBusy(old => old.filter(v => v === 'mutation'));
  }
  function tracked(lease: ReturnType<typeof begin>) {
    return { signal: lease.controller.signal, onTask: (record: CloudTaskRecord) => {
      // Original registrations survive view departure. No stop/cancel mutation is sent here.
      propsRef.current.onTaskRegistered?.(record);
      if (lease.live() && record.operation !== 'read_cloud_production') setTaskRecord(record);
    } };
  }
  function changeForm(apply: () => void) { apply(); setDirty(true); props.onUnappliedInputChanged?.(true); }
  function invalidateQuery(apply: () => void) {
    lanes.current.get('query')?.abort(); lanes.current.delete('query');
    setBusy(old => old.filter(v => v !== 'query')); setPage(null); apply();
  }
  async function configuration() {
    if (!context || !scenario || !available('read_cloud_production')) return;
    const lease = begin('configuration');
    try {
      const r = await readCloud(context, { kind: 'configuration', base_revision: context.projectRevision, scenario_id: scenario.id, scenario_revision: scenario.revision }, tracked(lease));
      if (lease.live() && r.kind === 'configuration') {
        if (r.data.project_revision !== context.projectRevision || r.data.scenario_id !== scenario.id || r.data.scenario_revision !== scenario.revision) throw new CloudProductionError('configuration_mismatch', '配置不属于所选工程/方案修订。');
        setConfig(r.data); setBindings(old => old.filter(b => r.data.candidate_bindings.some(c => c.binding_id === b)));
        if (r.data.cloud_connection_ref && r.data.cloud_project_id) void datasetCatalog(r.data);
        else setDatasets([]);
      }
    } catch (e) { fail(lease, e); } finally { finish(lease); }
  }
  async function datasetCatalog(configuration = config) {
    if (!context || !available('read_cloud_production') || !configuration?.cloud_connection_ref || !configuration.cloud_project_id) return;
    const lease = begin('datasets');
    try {
      const r = await readCloud(context, { kind: 'dataset_catalog' }, tracked(lease));
      if (lease.live() && r.kind === 'dataset_catalog') setDatasets(r.data.datasets);
    } catch (e) { fail(lease, e); } finally { finish(lease); }
  }
  async function localCatalog() {
    if (!context || !available('read_execution_view')) return;
    const lease = begin('catalog');
    try {
      const r = await readLocalView(context, { kind: 'catalog' });
      if (lease.live() && r.kind === 'catalog') {
        setCatalog(r.data.entries);
        const first = r.data.entries.find(e => e.active_owner) ?? r.data.entries[0];
        if (!selectedSource.current && first) await chooseSource({ identity: first.identity, origin: 'host', selection: first.active_owner ? { kind: 'latest' } : { kind: 'revision', execution_revision: first.execution_revision } });
      }
    } catch (e) { fail(lease, e); } finally { finish(lease); }
  }
  async function chooseSource(next: Source) {
    sourcePollingPaused.current = false; viewTimeEdited.current = false;
    selectedSource.current = next; sourceGeneration.current += 1;
    setSource(next); setEnvelope(null); setCommand(null); setReceipts(null); setReceiptPages(0); setPage(null);
    for (const lane of ['query', 'command', 'receipts'] as const) { lanes.current.get(lane)?.abort(); lanes.current.delete(lane); }
    setBusy(old => old.filter(v => !['query', 'command', 'receipts'].includes(v)));
    await refreshSource(next);
  }
  async function refreshSource(next = selectedSource.current, background = false) {
    if (!context || !next) return;
    if (background && sourcePollingPaused.current) return;
    if (!background) sourcePollingPaused.current = false;
    if (next.origin === 'host' && !available('get_execution_snapshot')) return;
    if (next.origin === 'cloud' && !available('read_cloud_production')) return;
    const lease = begin('source');
    try {
      let value: ExecutionReadEnvelope;
      if (next.origin === 'host') value = await readExecution(context, next.identity, next.selection);
      else {
        const r = await readCloud(context, { kind: 'snapshot', identity: next.identity, selection: next.selection }, tracked(lease));
        if (r.kind !== 'snapshot') throw new CloudProductionError('source_mismatch', '云端未返回所选快照。');
        value = r.data;
      }
      if (lease.live()) {
        if (identityKey(value.snapshot.identity) !== identityKey(next.identity) || value.delivery.read_origin !== next.origin
          || (next.selection.kind === 'revision' && value.snapshot.execution_revision !== next.selection.execution_revision)) throw new CloudProductionError('source_mismatch', '读取结果与当前固定来源不符。');
        setEnvelope(value);
        if (!viewTimeEdited.current) setViewTime(String(value.snapshot.coverage.observed_until_host_ms));
        setUntilMs(old => old || String(value.snapshot.coverage.observed_until_host_ms));
      }
    } catch (e) { fail(lease, e); } finally { finish(lease); }
  }
  useEffect(() => {
    mounted.current = true;
    sourcePollingPaused.current = false; viewTimeEdited.current = false;
    setError(''); setMessage(''); setConfig(null); setDatasets([]); setCatalog([]); setEnvelope(null); setSource(null); setPage(null); setTaskRecord(null);
    setCommand(null); setReceipts(null); setBusy([]); selectedSource.current = null; sourceGeneration.current += 1;
    const owner = `${hostContextId}:${project?.id ?? ''}:${scenario?.id ?? ''}`;
    if (previousFormOwner.current !== owner) {
      previousFormOwner.current = owner; setTarget(''); setUnit(''); setCutoff(''); setMaxAge(''); setBindings([]); setDirty(false);
      propsRef.current.onUnappliedInputChanged?.(false); setDataSet(''); setDeviceFilter(''); setUtcFrom(''); setUtcUntil(''); setFromMs('0'); setUntilMs(''); setPrediction('');
    }
    void configuration(); void localCatalog();
    return () => { mounted.current = false; for (const c of lanes.current.values()) c.abort(); lanes.current.clear(); };
    // Same-owner revision changes reload sources but preserve conflict form input.
  }, [key]);
  useEffect(() => {
    if (context && props.retainedTask && cloudContextKey(props.retainedTask.context) === cloudContextKey(context)) setTaskRecord(props.retainedTask);
  }, [key, props.retainedTask]);
  useEffect(() => {
    if (!source || !currentLatest || tab === 'data') return;
    if (envelope && ['completed', 'stopped', 'failed'].includes(envelope.snapshot.lifecycle.state) && envelope.delivery.relation === 'synced') return;
    const timer = window.setInterval(() => {
      const current = selectedSource.current;
      if (current?.selection.kind === 'latest' && !lanes.current.has('source') && !lanes.current.has('mutation') && !sourcePollingPaused.current) {
        void refreshSource(current, true);
      }
    }, 500);
    return () => window.clearInterval(timer);
    // Only latest device/execution views follow. Query pins and historical R/T stay fixed.
  }, [key, source, currentLatest, tab, envelope?.snapshot.lifecycle.state, envelope?.delivery.relation]);

  async function prepare() {
    if (!context || !scenario || !config || mutationBusy || unresolvedTask) return;
    const lease = begin('mutation'); const generation = sourceGeneration.current; setError(''); setMessage('');
    try {
      const chosen = config.candidate_bindings.filter(b => bindings.includes(b.binding_id));
      if (!config.cloud_connection_ref || !config.cloud_project_id) throw new CloudProductionError('not_configured', '需有效本机云配置。');
      if (chosen.length !== 2) throw new CloudProductionError('input', '本次演示需选择两台允许的候选设备。');
      const result = await prepareExecution(context, { execution_contract_version: EXECUTION_CONTRACT, base_revision: context.projectRevision,
        scenario_id: scenario.id, scenario_revision: scenario.revision, candidate_bindings: chosen,
        policy: { clock_mode: 'host_monotonic', report_cutoff_ms: safeInteger(cutoff, '截止毫秒', true), stop_new_dispatch_on_cutoff: true,
          finish_policy: 'drain_admitted_then_finish', target_quantity: parseQuantity(target, unit), source_prediction_window: { start_ms: 0, end_ms: scenario.window.end_ms },
          observation_max_age_ms: safeInteger(maxAge, '观测有效期', true) } }, tracked(lease));
      if (lease.live()) {
        setDirty(false); propsRef.current.onUnappliedInputChanged?.(false); setMessage('冻结输入已准备。尚未启动；请显式点击“启动”。');
        await propsRef.current.onRefreshProject?.();
        if (sourceGeneration.current === generation) await chooseSource({ identity: result.identity, origin: 'host', selection: { kind: 'latest' } });
        void localCatalog();
      }
    } catch (e) {
      fail(lease, e);
      if (lease.live() && cloudErrorCode(e) === 'RevisionConflict') {
        void refreshSource(); void configuration(); void propsRef.current.onRefreshProject?.();
      }
    } finally { finish(lease); }
  }
  async function control(action: ControlAction) {
    if (!context || !envelope || mutationBusy) return;
    const lease = begin('mutation'); const original = envelope; const generation = sourceGeneration.current; setError('');
    try {
      await controlExecution(context, original, action, currentLatest);
      if (lease.live()) { setMessage(`Host已确认 ${action.kind}。来源刷新结果按上方R/T显示。`); if (generation === sourceGeneration.current) await refreshSource(); }
    } catch (e) { fail(lease, e); if (lease.live() && generation === sourceGeneration.current) void refreshSource(); }
    finally { finish(lease); }
  }
  function makeQuery(): DaaSQuery {
    if (dataSet === '@execution' && snapshot) {
      const from = safeInteger(fromMs, 'Host起点'), until = safeInteger(untilMs, 'Host终点');
      if (from > until || until > snapshot.coverage.observed_until_host_ms) throw new CloudProductionError('range', 'Host区间需有序且不超过该R的实际观察前沿。');
      if (!['rows', 'accepted_quantity', 'command_counts', 'waiting_time'].includes(aggregation)) throw new CloudProductionError('aggregation', '请选择执行聚合。');
      return { dataset: 'execution', contract_version: EXECUTION_CONTRACT, identity: snapshot.identity, selection: { kind: 'revision', execution_revision: snapshot.execution_revision },
        host_from_ms: from, host_until_ms: until, aggregation: aggregation as 'rows' | 'accepted_quantity' | 'command_counts' | 'waiting_time', page: { limit: 50, cursor: null } };
    }
    const descriptor = datasets.find(d => d.dataset_id === dataSet);
    if (!descriptor) throw new CloudProductionError('dataset', '请选择实际数据集。');
    checkUtcRange(utcFrom, utcUntil);
    if (aggregation !== 'rows' && aggregation !== 'count_by_device_minute') throw new CloudProductionError('aggregation', '请选择遥测聚合。');
    return { dataset: 'telemetry', contract_version: EXECUTION_CONTRACT, cloud_project_id: descriptor.cloud_project_id, dataset_id: descriptor.dataset_id,
      device_ids: deviceFilter.split(',').map(s => s.trim()).filter(Boolean), received_from: utcFrom, received_until: utcUntil,
      aggregation, page: { limit: 50, cursor: null } };
  }
  async function queryData(nextPage = false) {
    if (!context) return;
    const lease = begin('query'); setError('');
    try {
      const previous = nextPage ? page : null;
      const query = previous ? { ...previous.query, page: { ...previous.query.page, cursor: previous.result.next_cursor } } : makeQuery();
      if (previous && previous.result.next_cursor === null) return;
      const r = await readCloud(context, { kind: 'data_query', query }, tracked(lease));
      if (lease.live() && r.kind === 'query_result') {
        const pinned = validateQueryPage(query, r.data, previous ? { selection: previous.pin, lastPosition: previous.lastPosition } : undefined,
          query.dataset === 'telemetry' ? datasets.find(d => d.dataset_id === query.dataset_id) : undefined);
        setPage({ query, result: r.data, pin: pinned.selection, lastPosition: pinned.lastPosition, number: (previous?.number ?? 0) + 1, seenRows: (previous?.seenRows ?? 0) + r.data.rows.length });
      }
    } catch (e) { fail(lease, e); } finally { finish(lease); }
  }
  async function inspectCommand(commandId: string, nextPage = false) {
    if (!context || !snapshot || !available('read_cloud_production')) return;
    const original = snapshot.identity;
    if (!nextPage) {
      setCommand(null); setReceipts(null); setReceiptPages(0);
      const lease = begin('command');
      try {
        const r = await readCloud(context, { kind: 'command', identity: original, command_id: commandId }, tracked(lease));
        if (!lease.live()) return;
        if (lease.live() && r.kind === 'command') {
          if (identityKey(r.data.command.identity) !== identityKey(original) || r.data.command.attempt.command_id !== commandId) throw new CloudProductionError('command_mismatch', '命令不属于所选来源。');
          setCommand(r.data);
        }
      } catch (e) { fail(lease, e); if (!lease.live()) return; } finally { finish(lease); }
    }
    const lease = begin('receipts');
    try {
      const r = await readCloud(context, { kind: 'command_receipts', query: { contract_version: EXECUTION_CONTRACT, identity: original, command_id: commandId,
        page: { limit: 50, cursor: nextPage ? receipts?.next_cursor ?? null : null } } }, tracked(lease));
      if (lease.live() && r.kind === 'command_receipts') {
        if (identityKey(r.data.identity) !== identityKey(original) || r.data.command_id !== commandId
          || (nextPage && receipts && (r.data.dataset_id !== receipts.dataset_id || r.data.visible_frontier !== receipts.visible_frontier))) throw new CloudProductionError('receipt_mismatch', '回执分页来源/前沿变化，未替换原页。');
        setReceipts(r.data); setReceiptPages(n => nextPage ? n + 1 : 1);
      }
    } catch (e) { fail(lease, e); } finally { finish(lease); }
  }
  async function exportResult(kind: 'report' | 'data') {
    if (!context || mutationBusy || unresolvedTask) return;
    const lease = begin('mutation'); setError(''); setMessage('');
    try {
      if (kind === 'data') {
        if (!page || !props.directories?.data) throw new CloudProductionError('directory', exportDirectoryUnavailable);
        const fixed = page;
        const destination = await props.directories.data(); lease.controller.signal.throwIfAborted(); if (!destination || !lease.live()) return;
        const result = await exportCloudData(context, { execution_contract_version: EXECUTION_CONTRACT, revision: context.projectRevision,
          query: exportQuery(fixed.query, fixed.pin), selection: fixed.pin, selected_destination_ref: destination }, tracked(lease));
        if (lease.live()) setMessage(`原固定筛选导出完成：${result.row_count}行、${result.page_count}页；见原Task ${result.export_id}。`);
      } else {
        if (!snapshot || !envelope || !props.directories?.report) throw new CloudProductionError('directory', exportDirectoryUnavailable);
        const fixed = { read_origin: envelope.delivery.read_origin, identity: snapshot.identity, execution_revision: snapshot.execution_revision };
        const chosenPrediction = props.predictionRuns?.find(r => r.run_id === prediction) ?? null;
        const destination = await props.directories.report(); lease.controller.signal.throwIfAborted(); if (!destination || !lease.live()) return;
        const result = await exportExecutionReport(context, { execution_contract_version: EXECUTION_CONTRACT, revision: context.projectRevision,
          sources: [fixed], formats: ['pdf', 'html', 'csv', 'json'], selected_destination_ref: destination,
          prediction: chosenPrediction ? { prediction: chosenPrediction, execution: fixed } : null }, tracked(lease));
        if (lease.live()) setMessage(`原来源报告Task完成：${result.report_id}；Host回执列出${result.files.length}个文件。`);
      }
    } catch (e) { fail(lease, e); } finally { finish(lease); }
  }
  async function recoverRequest() {
    if (!context || !taskRecord || !available('read_execution_view')) return;
    const original = taskRecord;
    if (cloudContextKey(original.context) !== cloudContextKey(context)) return;
    const lease = begin('request'); setError('');
    try {
      const result = await recoverRegisteredCloudTask(original, tracked(lease));
      if (!lease.live()) return;
      if (!result) { setMessage('原请求未登记，保持输入；未自动重复准备或导出。'); return; }
      if (lease.live()) { setMessage(`原Task读取完成：${result.kind}。未重复提交。`); if (result.kind === 'execution_session') void localCatalog(); }
    } catch (e) { fail(lease, e); } finally { finish(lease); }
  }
  function switchTab(next: Tab) { abortReads(); setTab(next); }
  function tabKey(e: KeyboardEvent<HTMLButtonElement>, index: number) {
    const next = e.key === 'Home' ? 0 : e.key === 'End' ? 2 : e.key === 'ArrowRight' ? (index + 1) % 3 : e.key === 'ArrowLeft' ? (index + 2) % 3 : null;
    if (next === null) return; e.preventDefault(); switchTab(tabs[next][0]); document.getElementById(`${id}-${tabs[next][0]}`)?.focus();
  }
  function inspectView() {
    if (!snapshot || !source) return;
    try { const t = safeInteger(viewTime, '回看时间'); if (t > snapshot.coverage.observed_until_host_ms) throw new CloudProductionError('time', '回看时间超过实际观察前沿。');
      props.onInspectExecution({ read_origin: source.origin, identity: snapshot.identity, execution_revision: snapshot.execution_revision, observed_until_host_ms: t });
    } catch (e) { setError(errorText(e)); }
  }
  const hasObligations = !!snapshot && (snapshot.lifecycle.unresolved_command_ids.length > 0 || snapshot.pending_reservations.length > 0 || snapshot.business_facts.pending_output_refs.length > 0);
  const configured = !!config?.cloud_connection_ref && !!config.cloud_project_id;
  const dataExportReason = !available('export_cloud_data') ? 'Host尚未公开筛选数据导出能力。' : !props.directories?.data ? exportDirectoryUnavailable : !page ? '先读取一页并固定来源。' : '';
  const reportReason = !available('export_execution_report') ? 'Host尚未公开执行报告能力。' : !props.directories?.report ? exportDirectoryUnavailable : !snapshot ? '先选择实际执行快照。' : '';
  return <section className="cloud-production" aria-label="云端生产工作区">
    <header className="cloud-heading"><div><h2>云端生产</h2><span>工业执行工作台 · {snapshot?.provenance.mode === 'physical_device' ? '实际物理设备来源' : '软件在环'}</span></div><button type="button" onClick={() => { abortReads(); props.onClose(); }}>返回画布</button></header>
    <div className="cloud-source" aria-label="当前来源">
      <span>工程 <strong>{project?.title ?? '未打开工程'}</strong> · 当前r{text(project?.revision)}</span>
      <span>冻结输入 {snapshot ? `工程r${snapshot.project_input_ref.project_revision} / 方案r${snapshot.project_input_ref.scenario_revision}` : '尚未选择'}</span>
      <span>{snapshot ? `${source?.origin === 'cloud' ? '云只读' : currentLatest ? 'Host当前来源' : 'Host归档 / 历史只读'} · R${snapshot.execution_revision} · T${snapshot.coverage.observed_until_host_ms}ms` : '尚未选择执行来源'}</span>
      <span>{envelope ? `云已发布R${text(envelope.delivery.cloud_published_revision)} · ${text(envelope.delivery.relation)}` : '尚无云发布记录'}</span>
      {snapshot && <details><summary>完整身份</summary><dl>{Object.entries(snapshot.identity).map(([k, v]) => <div key={k}><dt>{k}</dt><dd>{v}</dd></div>)}</dl><button type="button" onClick={() => { void navigator.clipboard.writeText(JSON.stringify(snapshot.identity)).then(() => setMessage('已复制完整执行身份。'), e => setError(errorText(e))); }}>复制完整身份</button></details>}
    </div>
    {error && <div className="cloud-alert" role="alert">{error}<button type="button" onClick={() => setError('')}>收起错误</button></div>}
    {message && <div className="cloud-message" role="status">{message}</div>}
    {busy.length > 0 && <p className="cloud-loading" role="status">正在处理：{busy.join(' / ')}。切换视图不停止生产。</p>}
    <div role="tablist" aria-label="云端生产页签" className="cloud-tabs">{tabs.map(([value, label], index) => <button type="button" role="tab" id={`${id}-${value}`} key={value} aria-selected={tab === value} aria-controls={`${id}-panel-${value}`} tabIndex={tab === value ? 0 : -1} onKeyDown={e => tabKey(e, index)} onClick={() => switchTab(value)}>{label}</button>)}</div>
    {!project || !scenario ? <Notice>请先打开工程并选择方案。面板没有创建演示数据或生产会话。</Notice> : <>
      <div className="cloud-source-picker"><label>本机执行来源<select aria-label="本机执行来源" value={source ? identityKey(source.identity) : ''} onChange={e => { const entry = catalog.find(c => identityKey(c.identity) === e.target.value); if (entry) void chooseSource({ identity: entry.identity, origin: 'host', selection: entry.active_owner ? { kind: 'latest' } : { kind: 'revision', execution_revision: entry.execution_revision } }); }}><option value="">尚未选择</option>{catalog.map(c => <option key={identityKey(c.identity)} value={identityKey(c.identity)}>{c.active_owner ? '当前' : '历史'} · {c.identity.session_id} / {c.identity.execution_epoch_id} · 目录R{c.execution_revision}</option>)}</select></label>
        <button type="button" disabled={!available('read_execution_view')} onClick={() => void localCatalog()}>刷新执行目录</button>
        <button type="button" disabled={!source} onClick={() => void refreshSource()}>刷新所选来源</button>
        <button type="button" disabled={!source || !snapshot || !available('read_cloud_production')} onClick={() => { if (source && snapshot) void chooseSource({ identity: source.identity, origin: 'cloud', selection: { kind: 'revision', execution_revision: snapshot.execution_revision } }); }}>云端同R只读</button>
        <button type="button" disabled={!source || !snapshot} onClick={() => { if (source && snapshot) void chooseSource({ identity: source.identity, origin: source.origin, selection: { kind: 'revision', execution_revision: snapshot.execution_revision } }); }}>固定当前R</button>
        <button type="button" disabled={!source || !available('get_execution_snapshot')} onClick={() => { if (source) void chooseSource({ identity: source.identity, origin: 'host', selection: { kind: 'latest' } }); }}>Host最新R</button>
        <label className="cloud-revision">指定修订R<input aria-label="指定修订R" value={revisionDraft} onChange={e => setRevisionDraft(e.target.value)}/></label>
        <button type="button" disabled={!source} onClick={() => { if (!source) return; try { const r = safeInteger(revisionDraft, '修订R'); void chooseSource({ identity: source.identity, origin: source.origin, selection: { kind: 'revision', execution_revision: r } }); } catch (e) { setError(errorText(e)); } }}>读取指定R</button>
      </div>
      <div role="tabpanel" id={`${id}-panel-${tab}`} aria-labelledby={`${id}-${tab}`} tabIndex={0} className="cloud-tab-content">
      {tab === 'devices' && <>
        <div className="cloud-section-heading"><h3>允许绑定与Host接纳观测</h3><button type="button" disabled={!available('read_cloud_production')} onClick={() => void configuration()}>刷新本机配置</button></div>
        {!available('read_cloud_production') && <Notice>当前Host尚未公开云端只读能力。</Notice>}
        {!configured && <Notice>云连接尚未配置或配置正在读取。凭据由Native受管配置提供，本页不收集token。</Notice>}
        <div className="cloud-table-scroll"><table><thead><tr><th>设备 / 精确绑定</th><th>人工允许</th><th>设备自报</th><th>Host新鲜度</th><th>有效资格 / 阻塞</th><th>操作</th></tr></thead><tbody>
          {(snapshot?.devices ?? []).map(d => <tr key={d.binding.binding_id}><td><strong>{d.binding.device_id}</strong><small>当前草稿设备名：{project.devices.find(p => p.id === d.binding.device_id)?.title ?? '无同ID设备'}<br/>{d.binding.device_module_ref.id}@{d.binding.device_module_ref.version}<br/>工艺 {d.binding.operation_ref.id}@{d.binding.operation_ref.version} / {d.binding.binding_id}</small></td><td>{d.operator_accept_new_work ? '允许' : '禁止'}</td><td>{d.latest_observation ? <>{d.latest_observation.observation.device_state}<small>自报接单：{d.latest_observation.observation.accept_new_work ? '是' : '否'}<br/>Host接纳：{d.latest_observation.host_received_elapsed_ms}ms</small></> : '未知'}</td><td>{labels[d.freshness] ?? d.freshness}</td><td>{d.effective_accept_new_work === null ? '未知' : d.effective_accept_new_work ? '可接新工作' : '不可接新工作'}<small>{d.blocking_reasons.join('；') || '无已报告阻塞'}</small></td><td><button type="button" onClick={() => props.onLocateDevice(d.binding.device_id)}>定位</button><button type="button" disabled={mutationBusy || !available('control_execution_session') || !canControl(envelope, { kind: 'set_device_acceptance', device_id: d.binding.device_id, accept_new_work: !d.operator_accept_new_work }, currentLatest)} onClick={() => void control({ kind: 'set_device_acceptance', device_id: d.binding.device_id, accept_new_work: !d.operator_accept_new_work })}>{d.operator_accept_new_work ? '禁止新接单' : '允许新接单'}</button></td></tr>)}
          {!snapshot?.devices.length && config?.candidate_bindings.map(b => <tr key={b.binding_id}><td>{project.devices.find(p => p.id === b.device_id)?.title ?? b.device_id}<small>{b.device_module_ref.id}@{b.device_module_ref.version} / {b.binding_id}</small></td><td colSpan={4}>仅配置候选；尚无所选R的Host接纳观测，资格未知。</td><td><button type="button" onClick={() => props.onLocateDevice(b.device_id)}>定位</button></td></tr>)}
        </tbody></table></div>
        {!snapshot?.devices.length && !config?.candidate_bindings.length && <Notice>尚无可显示的实际设备绑定。</Notice>}
        {config && <><Json value={config.issues} label={`配置问题（${config.issues.length}）`}/><Json value={config} label="本机配置完整字段"/></>}
      </>}
      {tab === 'data' && <>
        <div className="cloud-section-heading"><h3>固定筛选数据</h3><button type="button" disabled={!configured || !available('read_cloud_production')} onClick={() => void datasetCatalog()}>刷新数据集</button></div>
        <form className="cloud-filter" onSubmit={e => { e.preventDefault(); void queryData(); }}>
          <label>数据集<select aria-label="数据集" value={dataSet} onChange={e => invalidateQuery(() => { setDataSet(e.target.value); setAggregation('rows'); })}><option value="">请选择实际数据集</option>{datasets.map(d => <option key={`${d.cloud_project_id}:${d.dataset_id}`} value={d.dataset_id}>{d.dataset_id} · {d.purpose} · {d.source_mode}</option>)}{snapshot && <option value="@execution">所选执行投影 · R{snapshot.execution_revision}</option>}</select></label>
          <label>聚合<select aria-label="聚合" value={aggregation} onChange={e => invalidateQuery(() => setAggregation(e.target.value))}><option value="rows">原始行</option>{dataSet === '@execution' ? <><option value="accepted_quantity">接受量</option><option value="command_counts">命令状态数量</option><option value="waiting_time">等待时间</option></> : <option value="count_by_device_minute">设备 / 分钟计数</option>}</select></label>
          {dataSet === '@execution' ? <><label>Host起点ms<input aria-label="Host起点ms" value={fromMs} onChange={e => invalidateQuery(() => setFromMs(e.target.value))}/></label><label>Host终点ms<input aria-label="Host终点ms" value={untilMs} onChange={e => invalidateQuery(() => setUntilMs(e.target.value))}/></label></> : <><label>设备ID（逗号分隔；空为Host固定集合）<input aria-label="设备筛选" value={deviceFilter} onChange={e => invalidateQuery(() => setDeviceFilter(e.target.value))}/></label><label>UTC起点（含）<input aria-label="UTC起点" placeholder="2026-10-27T00:00:00.000001Z" value={utcFrom} onChange={e => invalidateQuery(() => setUtcFrom(e.target.value))}/></label><label>UTC终点（不含）<input aria-label="UTC终点" placeholder="2026-10-27T01:00:00.000001Z" value={utcUntil} onChange={e => invalidateQuery(() => setUtcUntil(e.target.value))}/></label></>}
          <button type="submit" disabled={!dataSet || !available('read_cloud_production') || busy.includes('query')}>重新查询并固定来源</button>
        </form>
        {!page ? <Notice>尚未取得当前筛选结果。数据集目录的前沿不代表查询前沿；首响应才固定F/设备集合或R。</Notice> : <>
          <div className="cloud-page-status" role="status">第{page.number}页 · 当前页{page.result.rows.length}行 · 本分页链已读{page.seenRows}行 · {page.result.next_cursor === null ? '已到末页' : '还有后续页，当前页不代表全量'}</div>
          <Json value={{ query: page.query, selection: page.pin }} label={page.pin.dataset === 'telemetry' ? `固定F=${page.pin.selected_visible_frontier} / ${page.pin.selected_device_ids.length}个设备；查看完整筛选` : `固定执行R=${page.pin.execution_revision}；查看完整筛选`}/>
          <div className="cloud-table-scroll"><table><thead><tr><th>位置</th><th>类型</th><th>事实</th></tr></thead><tbody>{page.result.rows.map((row, index) => <tr key={index}><td>{'ingress_id' in row ? row.ingress_id : row.row_index}</td><td>{row.kind}</td><td>{'cloud_received_at' in row && <small>云接收 {row.cloud_received_at}</small>}{'quantity' in row && <strong>{formatQuantity(row.quantity)}</strong>}{'count' in row && <strong>{row.count}条（该聚合行）</strong>}<Json value={row} label="查看完整行"/></td></tr>)}</tbody></table></div>
          {page.result.rows.length === 0 && <Notice>该固定筛选当前为空；空页及F=0仍是有效来源。</Notice>}
          <Json value={page.result.data_quality_issues} label={`数据质量问题（${page.result.data_quality_issues.length}）`}/>
          <button type="button" disabled={page.result.next_cursor === null || busy.includes('query')} onClick={() => void queryData(true)}>读取下一页</button>
        </>}
        <div className="cloud-export"><button type="button" title={dataExportReason} disabled={!!dataExportReason || mutationBusy || unresolvedTask} onClick={() => void exportResult('data')}>导出全部固定筛选</button><span>{dataExportReason || '导出完整固定来源；包含所有分页，不只是当前页。'}</span></div>
      </>}
      {tab === 'execution' && <>
        <div className="cloud-section-heading"><h3>冻结输入与生产控制</h3><span>{dirty ? '表单有未应用输入' : '准备不会自动启动'}</span></div>
        <form className="cloud-prepare" onSubmit={e => { e.preventDefault(); void prepare(); }}>
          <fieldset disabled={mutationBusy}><legend>新会话冻结输入 · 工程r{project.revision} · {scenario.title} r{scenario.revision}</legend>
            <div className="cloud-field-grid"><label>目标数量<input aria-label="目标数量" value={target} onChange={e => changeForm(() => setTarget(e.target.value))}/></label><label>目标单位<input aria-label="目标单位" value={unit} onChange={e => changeForm(() => setUnit(e.target.value))}/></label><label>报告截止ms<input aria-label="报告截止ms" value={cutoff} onChange={e => changeForm(() => setCutoff(e.target.value))}/></label><label>观测有效期ms<input aria-label="观测有效期ms" value={maxAge} onChange={e => changeForm(() => setMaxAge(e.target.value))}/></label></div>
            <p>本表单准备新的会话，不修改已选会话的冻结输入。Host单调时钟；截止后停止新派工，已准入工作收尾。原预测窗口 [0, {scenario.window.end_ms})ms。</p>
            <div className="cloud-bindings">{config?.candidate_bindings.map(b => <label key={b.binding_id}><input type="checkbox" checked={bindings.includes(b.binding_id)} onChange={e => changeForm(() => setBindings(old => e.target.checked ? [...old, b.binding_id] : old.filter(v => v !== b.binding_id)))}/>{b.binding_id} · {b.device_id} · {b.operation_ref.id}@{b.operation_ref.version}</label>)}</div>
            <button type="submit" disabled={!configured || !available('start_execution_session') || unresolvedTask}>准备冻结输入</button>
            {!available('start_execution_session') && <small>Host尚未公开准备能力。</small>}
          </fieldset>
        </form>
        <div className="cloud-controls">{([['run', '启动'], ['pause_dispatch', '暂停新派工'], ['resume_dispatch', '继续新派工'], ['stop_dispatch', '停止新派工'], ['request_reconciliation', '请求原命令核对']] as const).map(([kind, label]) => <button type="button" key={kind} disabled={mutationBusy || !available('control_execution_session') || !canControl(envelope, { kind }, currentLatest)} onClick={() => void control({ kind })}>{label}</button>)}</div>
        {!currentLatest && <Notice>历史修订、其他Host与云来源只读。控制权限以实际Host access为准。</Notice>}
        {!snapshot ? <Notice>尚无执行快照；不会用当前草稿或旧Run代替执行来源。</Notice> : <>
          <div className="cloud-facts"><div><span>生命周期</span><strong>{labels[snapshot.lifecycle.state] ?? snapshot.lifecycle.state}</strong></div><div><span>目标 {formatQuantity(snapshot.policy.target_quantity)}</span><strong>本时点：{labels[snapshot.goal_at_snapshot]}</strong><small>截止T={snapshot.policy.report_cutoff_ms}ms：{labels[snapshot.goal_at_cutoff]}</small></div><div><span>已准入未决 / 待输出 / 预留</span><strong>{snapshot.lifecycle.unresolved_command_ids.length} / {snapshot.business_facts.pending_output_refs.length} / {snapshot.pending_reservations.length}</strong></div></div>
          {hasObligations && <Notice>仍有在途或待核对义务。目标满足也不代表生产已完成。</Notice>}
          <h4>同R接受事件（{snapshot.business_facts.acceptance_events.length}）</h4><div className="cloud-table-scroll"><table><thead><tr><th>Host时间</th><th>接受量</th><th>去向</th><th>阶段</th></tr></thead><tbody>{snapshot.business_facts.acceptance_events.map((a, i) => <tr key={i}><td>{a.at_ms}ms</td><td>{formatQuantity(a.quantity)}</td><td>{a.destination}</td><td>{a.acceptance_stage}</td></tr>)}</tbody></table></div>
          <h4>逻辑工作与派工理由（{snapshot.logical_work.length}）</h4>{snapshot.logical_work.map(w => <div className="cloud-record" key={w.logical_job_id}><strong>{w.logical_job_id}</strong> · {w.state} · 工单 {w.production_task_id} / 周期 {w.cycle}<Json value={w.latest_dispatch_decision} label="Host派工理由与候选"/></div>)}
          <h4>尝试与命令（{snapshot.attempts.length}）</h4>{snapshot.attempts.map(a => <div className="cloud-record" key={a.attempt.command_id}><strong>{a.attempt.command_id}</strong> · {a.device_id} · {a.state}<small>{a.attempt.logical_job_id} / 第{a.attempt.attempt_ordinal}次尝试</small><Json value={a} label="尝试及调度事实"/><button type="button" disabled={!available('read_cloud_production')} onClick={() => void inspectCommand(a.attempt.command_id)}>查看命令与云原始回执</button></div>)}
          <Json value={snapshot.receipt_admissions} label={`同R Host接纳决定（${snapshot.receipt_admissions.length}）；与云原始回执分列`}/>
          <h4>同R工时（缺测保持未知）</h4><div className="cloud-table-scroll"><table><thead><tr><th>工单 / 命令</th><th>名义ms</th><th>Host预留ms</th><th>Host占用ms</th><th>设备自报ms</th><th>物理有效ms</th></tr></thead><tbody>{snapshot.timings.map(t => <tr key={`${t.job_id}:${t.command_id}`}><td>{t.job_id}<small>{t.command_id}</small><Json value={t.missing_conditions} label="缺失条件"/></td><td>{text(t.nominal_work_ms)}</td><td>{text(t.host_confirmed_reservation_ms)}</td><td>{text(t.host_confirmed_occupation_ms)}</td><td>{text(t.device_reported_duration_ms)}</td><td>{text(t.physical_effective_work_ms)}</td></tr>)}</tbody></table></div>
          {command && <Json value={command} label={`原始命令与投递/Host确认：${command.command.attempt.command_id}`}/>}
          {receipts && <div className="cloud-record"><h4>云原始回执 · F{receipts.visible_frontier} · 第{receiptPages}页</h4><p>这是独立云流前缀，非同R接纳结果；{receipts.next_cursor ? '还有后续页' : '该前缀已到末页'}。</p><Json value={receipts.rows} label={`当前页${receipts.rows.length}条原始回执`}/><button type="button" disabled={!receipts.next_cursor || busy.includes('receipts')} onClick={() => void inspectCommand(receipts.command_id, true)}>下一页原始回执</button></div>}
          <div className="cloud-view-action"><label>固定R{snapshot.execution_revision}回看T(ms)<input aria-label="回看T毫秒" value={viewTime} onChange={e => { viewTimeEdited.current = true; setViewTime(e.target.value); }}/></label><button type="button" onClick={inspectView}>回看执行画布与时间轴</button></div>
          <Json value={snapshot.coverage.data_quality_issues} label="执行覆盖与质量问题"/>
        </>}
        <div className="cloud-export"><label>实际预测Run（可不选）<select aria-label="预测Run" value={prediction} onChange={e => setPrediction(e.target.value)}><option value="">不比较预测</option>{props.predictionRuns?.map(r => <option key={r.run_id} value={r.run_id}>{r.run_id} · 工程r{r.input_ref.project_revision} / 方案r{r.input_ref.scenario_revision}</option>)}</select></label><button type="button" title={reportReason} disabled={!!reportReason || mutationBusy || unresolvedTask} onClick={() => void exportResult('report')}>导出同源四格式报告</button><span>{reportReason}</span></div>
      </>}
      </div>
    </>}
    {taskRecord && <footer className="cloud-task"><strong>原任务观察</strong><span>{taskRecord.operation} · {taskRecord.status}</span><code>{taskRecord.request_id}</code><code>{taskRecord.task?.id ?? '登记回执尚未确认'}</code><button type="button" disabled={!available('read_execution_view') || busy.includes('request') || mutationBusy} onClick={() => void recoverRequest()}>查询原请求 / 继续观察</button><small>{unresolvedTask ? '原任务状态未消歧，准备和导出暂禁用；请查询原请求。' : ''}关闭或切换只停止观察；已登记准备/导出义务继续，生产不会被停止。</small></footer>}
  </section>;
}
