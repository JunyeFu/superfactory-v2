import { useEffect, useId, useRef, useState } from 'react';
import { FactoryScene } from '../../scene/FactoryScene';
import type { LodMode } from '../../scene/FactoryScene';
import { cloudErrorCode, formatQuantity, identityKey } from '../../services/cloudProduction';
import type { CloudContext } from '../../services/cloudProduction';
import {
  createExecutionPresentation, executionBaseKey, executionObjectRef, executionSourceKey,
  readCompleteExecutionHistory, readCompleteExecutionObservation, readExecutionFrozenSource,
} from '../../services/executionPresentation';
import type { CompleteExecutionHistory, CompleteExecutionObservation, ExecutionAssetPort,
  ExecutionFrozenSource, ExecutionObservationSelection } from '../../services/executionPresentation';
import type { FactoryPresentationSource } from '../../services/factoryPresentation';
import type { ObjectRef } from '../../../../../packages/commands/generated/typescript/ObjectRef';
import type { ExecutionObservedObject } from '../../../../../packages/commands/generated/typescript/ExecutionObservedObject';
import type { ExecutionObservedObjectRef } from '../../../../../packages/commands/generated/typescript/ExecutionObservedObjectRef';
import './executionObservation.css';

export interface ExecutionObservationViewProps {
  context: CloudContext;
  /** Actual caller Native UI session, not an execution epoch or fabricated Run ID. */
  nativeSessionId: string;
  selection: ExecutionObservationSelection;
  readAsset?: ExecutionAssetPort;
  onClose: () => void;
  onLocateObject: (reference: ObjectRef | ExecutionObservedObjectRef) => void;
  onSelectionChange?: (selection: ExecutionObservationSelection) => void;
}
const kinds = [['item', '物品'], ['job', '绑定作业'], ['device', '设备'], ['logical_work', '逻辑工作'], ['attempt', '尝试']] as const;
const label = (kind: ExecutionObservedObject['kind']) => kinds.find(k => k[0] === kind)![1];
const json = (value: unknown) => JSON.stringify(value, null, 2);
function detail(object: ExecutionObservedObject) {
  switch (object.kind) {
    case 'item': return `${formatQuantity(object.data.quantity)} · ${object.data.lifecycle} / ${object.data.quality} · holder ${object.data.holder.kind}:${object.data.holder.id}`;
    case 'job': return `${object.data.state} · 阶段 ${object.data.phase_id ?? '未知'} · Started ${object.data.started_at_host_ms}ms · 完成 ${object.data.finished_at_host_ms ?? '未知'} · 原物品 ${object.data.input_refs.join('、')}`;
    case 'device': return `人工${object.data.operator_accept_new_work ? '允许' : '禁止'} · 自报 ${object.data.latest_observation?.observation.device_state ?? '未知'} · Host ${object.data.freshness} · ${object.data.blocking_reasons.join('；')}`;
    case 'logical_work': return `${object.data.state} · 工单 ${object.data.production_task_id} · 决策 ${object.data.latest_dispatch_decision?.reason ?? '未知'}`;
    case 'attempt': return `${object.data.state} · 设备 ${object.data.device_id} · 尝试 ${object.data.attempt.attempt_ordinal} · Host回执 ${object.data.receipt_ids.join('、') || '无'}`;
  }
}
function integer(raw: string) { const n = Number(raw); if (!/^\d+$/.test(raw) || !Number.isSafeInteger(n)) throw new Error('请输入非负整数毫秒。'); return n; }

export function ExecutionObservationView(props: ExecutionObservationViewProps) {
  const { context, nativeSessionId, selection } = props;
  const baseKey = executionBaseKey(context, nativeSessionId, selection.identity, selection.execution_revision, selection.read_origin);
  const activeKey = useRef(baseKey); activeKey.current = baseKey;
  const propsRef = useRef(props); propsRef.current = props;
  const inputTicket = useRef<AbortController | null>(null), objectTicket = useRef<AbortController | null>(null), historyTicket = useRef<AbortController | null>(null);
  const [frozen, setFrozen] = useState<{ key: string; value: ExecutionFrozenSource } | null>(null);
  const [at, setAt] = useState(selection.observed_until_host_ms), [timeDraft, setTimeDraft] = useState(String(at));
  const [observation, setObservation] = useState<{ key: string; value: CompleteExecutionObservation; scene: FactoryPresentationSource } | null>(null);
  const [history, setHistory] = useState<{ key: string; value: CompleteExecutionHistory } | null>(null);
  const [range, setRange] = useState<{ from: number; until: number } | null>(null), [rangeFrom, setRangeFrom] = useState('0'), [rangeUntil, setRangeUntil] = useState('');
  const [error, setError] = useState(''), [loading, setLoading] = useState<string[]>([]);
  const [mediaNotice, setMediaNotice] = useState('');
  const [canvas, setCanvas] = useState<'line_canvas' | 'multi_hall_canvas'>('line_canvas');
  const [hallId, setHallId] = useState(''), [lod, setLod] = useState<LodMode>('detailed');
  const [sceneSelection, setSceneSelection] = useState<ObjectRef | null>(null), [reset, setReset] = useState(0);
  const [kind, setKind] = useState('all'), [objectId, setObjectId] = useState('');
  const id = useId();
  const fixed = frozen?.key === baseKey ? frozen.value : null;
  const targetKey = fixed ? executionSourceKey(fixed, nativeSessionId, at) : '';
  const targetRef = useRef(targetKey); targetRef.current = targetKey;
  const current = observation?.key === targetKey ? observation : null;
  const historyKey = range && fixed ? JSON.stringify([executionSourceKey(fixed, nativeSessionId, range.until), range.from, range.until]) : '';
  const rangeRef = useRef(historyKey); rangeRef.current = historyKey;
  const currentHistory = history?.key === historyKey ? history.value : null;
  const front = fixed?.envelope.snapshot.coverage.observed_until_host_ms ?? 0;

  function busy(name: string, value: boolean) { setLoading(old => value ? [...old.filter(v => v !== name), name] : old.filter(v => v !== name)); }
  function showError(reason: unknown, stillCurrent: () => boolean) {
    if (!stillCurrent()) return;
    const code = cloudErrorCode(reason);
    if (code === 'PermissionDenied' || code === 'NoActiveProject') {
      inputTicket.current?.abort(); objectTicket.current?.abort(); historyTicket.current?.abort();
      setFrozen(null); setObservation(null); setHistory(null); setLoading([]); setMediaNotice('');
    }
    setError(reason instanceof Error ? reason.message : String(reason));
  }
  useEffect(() => {
    const controller = new AbortController(); inputTicket.current = controller;
    const live = () => activeKey.current === baseKey && !controller.signal.aborted;
    setFrozen(null); setObservation(null); setHistory(null); setRange(null); setSceneSelection(null); setError(''); setMediaNotice(''); setLoading(['冻结输入']);
    setAt(selection.observed_until_host_ms); setTimeDraft(String(selection.observed_until_host_ms)); setKind('all'); setObjectId('');
    void readExecutionFrozenSource(context, selection.identity, selection.execution_revision, selection.read_origin, { signal: controller.signal }).then(value => {
      if (!live()) return;
      setFrozen({ key: baseKey, value }); setHallId(value.inputs.project.halls[0]?.id ?? '');
      const until = value.envelope.snapshot.coverage.observed_until_host_ms;
      setRange({ from: 0, until }); setRangeFrom('0'); setRangeUntil(String(until));
    }).catch(e => showError(e, live)).finally(() => { if (live()) busy('冻结输入', false); });
    return () => { controller.abort(); objectTicket.current?.abort(); historyTicket.current?.abort(); };
  }, [baseKey]);
  useEffect(() => {
    setAt(selection.observed_until_host_ms); setTimeDraft(String(selection.observed_until_host_ms));
  }, [baseKey, selection.observed_until_host_ms]);
  useEffect(() => {
    if (!fixed) return;
    const controller = new AbortController(); objectTicket.current = controller;
    const expected = targetKey;
    const live = () => activeKey.current === baseKey && targetRef.current === expected && !controller.signal.aborted;
    const options = { signal: controller.signal, assertCurrent: () => { if (!live()) throw new DOMException('选择已变更', 'AbortError'); } };
    setObservation(null); busy('完整时点对象', true); setError(''); setMediaNotice('');
    void (async () => {
      const value = await readCompleteExecutionObservation(fixed, at, options);
      const port = propsRef.current.readAsset;
      const protectedPort: ExecutionAssetPort | undefined = port ? async request => {
        try { return await port(request); }
        catch (reason) {
          if (cloudErrorCode(reason) === 'MissingReference') {
            if (live()) setMediaNotice(`所选来源的冻结模型在本机不可用；已读取的对象事实和历史仍可查看。${reason instanceof Error ? reason.message : String(reason)}`);
          } else showError(reason, live);
          throw reason;
        }
      } : undefined;
      const scene = await createExecutionPresentation(fixed, value, nativeSessionId, protectedPort, options);
      if (live()) setObservation({ key: expected, value, scene });
    })().catch(e => showError(e, live)).finally(() => { if (live()) busy('完整时点对象', false); });
    return () => controller.abort();
  }, [baseKey, targetKey, !!props.readAsset]);
  useEffect(() => {
    if (!fixed || !range) return;
    const controller = new AbortController(); historyTicket.current = controller;
    const expected = historyKey;
    const live = () => activeKey.current === baseKey && rangeRef.current === expected && !controller.signal.aborted;
    setHistory(null); busy('完整历史', true);
    void readCompleteExecutionHistory(fixed, range.from, range.until, { signal: controller.signal }).then(value => {
      if (live()) setHistory({ key: expected, value });
    }).catch(e => showError(e, live)).finally(() => { if (live()) busy('完整历史', false); });
    return () => controller.abort();
  }, [baseKey, fixed, historyKey]);
  function seek(next: number) {
    if (!fixed || next > front || next < 0) { setError('时点超出固定R的实际观察前沿。'); return; }
    setAt(next); setTimeDraft(String(next)); props.onSelectionChange?.({ ...selection, observed_until_host_ms: next });
  }
  function locate(reference: ObjectRef | ExecutionObservedObjectRef) {
    if (reference.kind === 'item' || reference.kind === 'device' || reference.kind === 'job') setSceneSelection({ kind: reference.kind, id: reference.id });
    props.onLocateObject(reference);
  }
  const accepts = (object: ExecutionObservedObject) => (kind === 'all' || object.kind === kind) && (!objectId || executionObjectRef(object).id === objectId);
  const objects = current?.value.objects.filter(accepts) ?? [], records = currentHistory?.records.filter(r => accepts(r.object)) ?? [];
  return <section className="execution-observation" aria-label="执行只读观察工作区">
    <header className="execution-view-heading"><div><h2>执行观察</h2><span>{selection.read_origin === 'cloud' ? '云端只读归档' : 'Host只读回看（含归档）'} · {fixed ? fixed.envelope.snapshot.provenance.mode === 'physical_device' ? '物理设备来源' : '软件在环' : '尚未读取来源事实'} · 物理进度未测</span></div><button type="button" onClick={() => { inputTicket.current?.abort(); objectTicket.current?.abort(); historyTicket.current?.abort(); props.onClose(); }}>返回云端生产</button></header>
    <div className="execution-view-source" aria-label="执行观察来源"><span className="execution-source-origin">read_origin={selection.read_origin} · ownership={fixed?.envelope.access.ownership ?? '未知'} · 本页只读</span><span>R{selection.execution_revision} · F{fixed?.envelope.snapshot.observation_frontier ?? '未知'} · T={at}ms</span><span>冻结工程r{fixed?.inputs.project.revision ?? '未知'} / 方案r{fixed?.inputs.scenario.revision ?? '未知'}</span><span>clock_started={fixed ? String(fixed.envelope.snapshot.coverage.clock_started) : '未知'} · 前沿{fixed ? `${front}ms` : '未知'}</span><details><summary>完整来源身份</summary><pre>{json({ read_origin: selection.read_origin, native_host_instance_id: context.hostContextId, native_ui_session_id: nativeSessionId, ...selection.identity, execution_revision: selection.execution_revision, observation_frontier: fixed?.envelope.snapshot.observation_frontier ?? null, at_host_ms: at, input_ref: fixed?.inputs.project_input_ref ?? null })}</pre><button type="button" onClick={() => { void navigator.clipboard.writeText(json({ read_origin: selection.read_origin, identity: selection.identity, execution_revision: selection.execution_revision, observed_until_host_ms: at })).catch(e => setError(String(e))); }}>复制完整来源</button></details></div>
    {error && <p role="alert" className="execution-view-error">{error} 本次未取得完整新视图；没有使用草稿或别的Run补齐。</p>}
    {loading.length > 0 && <p role="status" className="execution-view-loading">读取中：{loading.join(' / ')}。历史不会自动跟随新修订。</p>}
    <div className="execution-view-controls">
      <label>视角<select aria-label="观察视角" value={canvas} onChange={e => setCanvas(e.target.value as typeof canvas)}><option value="line_canvas">单厂房 / 流水线</option><option value="multi_hall_canvas">多厂房</option></select></label>
      <label>冻结厂房<select aria-label="冻结厂房" value={hallId} onChange={e => setHallId(e.target.value)}>{fixed?.inputs.project.halls.map(h => <option key={h.id} value={h.id}>{h.title} · {h.id}</option>)}</select></label>
      <label>只读LOD<select aria-label="观察LOD" value={lod} onChange={e => setLod(e.target.value as LodMode)}><option value="detailed">详细声明几何</option><option value="simplified">简化声明几何</option><option value="region">区域布局</option></select></label>
      <button type="button" disabled={!current} onClick={() => setReset(v => v + 1)}>重置视角</button>
      <form onSubmit={e => { e.preventDefault(); try { seek(integer(timeDraft)); } catch (reason) { setError(String(reason)); } }}><label>固定R内T(ms)<input aria-label="固定R内T" value={timeDraft} onChange={e => setTimeDraft(e.target.value)}/></label><button type="submit" disabled={!fixed}>读取时点</button></form>
    </div>
    <div className="execution-view-clock"><input aria-label="执行观察时间游标" type="range" min={0} max={front} step={1} disabled={!fixed || !fixed.envelope.snapshot.coverage.clock_started} value={Math.min(at, front)} onChange={e => seek(Number(e.target.value))}/><span>{fixed && !fixed.envelope.snapshot.coverage.clock_started ? 'prepared：T=0初始化；尚未开始生产。' : `T=${at}ms；不推进Host时钟、不推算名义进度。`}</span></div>
    {!props.readAsset && <p className="execution-view-notice">执行归档媒体字节端口尚未接入；只读冻结布局可用，精确模型不可用。不从当前模块或任意URL取代。</p>}
    {mediaNotice && <p className="execution-view-notice" role="status">{mediaNotice}</p>}
    <div className="execution-view-scene" aria-busy={!current}>{current ? <FactoryScene key={baseKey} project={fixed!.inputs.project} canvas={canvas} hallId={hallId || null} selection={sceneSelection} onSelect={ref => { setSceneSelection(ref); if (ref) props.onLocateObject(ref); }} lod={lod} autoLod={false} resetKey={reset} mediaSource={current.scene} modules={current.scene.modules} run={null} observation={null}/> : <p className="execution-view-empty">{error ? '本选择的三维场景不可用。' : '正在取得完整同R/F/T对象；不显示旧时点场景。'}</p>}</div>
    {current && <p className="execution-view-complete" role="status">时点对象完整读取：{current.value.objects.length}对象 / {current.value.pages}页。三维为冻结声明几何；实际holder和阶段见下方同T事实，设备物理进度及未知资源旅行不推测。</p>}
    <div className="execution-view-filter"><label>对象类别<select aria-label="观察对象类别" value={kind} onChange={e => setKind(e.target.value)}><option value="all">全部五类</option>{kinds.map(([value, title]) => <option key={value} value={value}>{title}</option>)}</select></label><label>精确对象ID（空为全部）<input aria-label="观察对象ID" value={objectId} onChange={e => setObjectId(e.target.value)}/></label><button type="button" onClick={() => { setKind('all'); setObjectId(''); }}>清除筛选</button></div>
    <div className="execution-view-table"><table><caption>固定T对象事实；缺少观察不从最终快照反推</caption><thead><tr><th>对象</th><th>实际已知状态</th><th>详情</th></tr></thead><tbody>{objects.map(object => { const ref = executionObjectRef(object); return <tr key={`${ref.kind}:${ref.id}`}><td><button type="button" onClick={() => locate(ref)}>{label(object.kind)} · {ref.id}</button></td><td>{detail(object)}</td><td><details><summary>原始字段</summary><pre>{json(object.data)}</pre></details></td></tr>; })}</tbody></table>{current && objects.length === 0 && <p>当前筛选没有对象记录；不代表其它对象不存在。</p>}</div>
    <div className="execution-history-heading"><h3>同R/F时间轴与原历史</h3><form onSubmit={e => { e.preventDefault(); try { const from = integer(rangeFrom), until = integer(rangeUntil); if (from > until || until > front) throw new Error('历史闭区间超出该R观察前沿。'); setRange({ from, until }); } catch (reason) { setError(String(reason)); } }}><label>闭区间起点ms<input aria-label="历史起点" value={rangeFrom} onChange={e => setRangeFrom(e.target.value)}/></label><label>闭区间终点ms<input aria-label="历史终点" value={rangeUntil} onChange={e => setRangeUntil(e.target.value)}/></label><button type="submit" disabled={!fixed}>读取完整历史</button></form></div>
    <div className="execution-history-axis" aria-label="同R历史主轴"><span>{range?.from ?? 0}ms</span><div>{records.map(r => <button type="button" key={r.record_sequence} style={{ left: `${range && range.until !== range.from ? 100 * (r.at_host_ms - range.from) / (range.until - range.from) : 0}%` }} title={`#${r.record_sequence} · ${r.at_host_ms}ms · ${executionObjectRef(r.object).id}`} aria-label={`跳到记录${r.record_sequence}`} onClick={() => seek(r.at_host_ms)}/>)}</div><span>{range?.until ?? 0}ms</span></div>
    {currentHistory && <p className="execution-view-complete" role="status">历史完整读取：{currentHistory.records.length}记录 / {currentHistory.pages}页 · 闭区间[{currentHistory.host_from_ms}, {currentHistory.host_until_ms}]ms · 按原record_sequence，同毫秒不重排。游标T之后的记录明确标注。</p>}
    <div className="execution-view-table"><table id={`${id}-history`}><thead><tr><th>原序号 / R</th><th>Host时间</th><th>对象及因果状态</th></tr></thead><tbody>{records.map(r => { const ref = executionObjectRef(r.object); return <tr key={r.record_sequence} data-record-sequence={r.record_sequence} className={r.at_host_ms > at ? 'execution-history-future' : ''}><td>#{r.record_sequence} · R{r.execution_revision}</td><td><button type="button" onClick={() => seek(r.at_host_ms)}>{r.at_host_ms}ms</button>{r.at_host_ms > at && <small>T之后</small>}</td><td><button type="button" onClick={() => locate(ref)}>{label(r.object.kind)} · {ref.id}</button><p>{detail(r.object)}</p><details><summary>原始历史记录</summary><pre>{json(r)}</pre></details></td></tr>; })}</tbody></table>{currentHistory && records.length === 0 && <p>这个闭区间及对象筛选没有历史记录。</p>}{!currentHistory && <p>未取得完整历史；分页失败不标为完成。</p>}</div>
  </section>;
}
