import { useEffect, useMemo, useRef, useState } from 'react';
import type { OperationName } from '../../../../../packages/commands/generated/typescript/OperationName';
import type { FilePurpose } from '../../../../../packages/commands/generated/typescript/FilePurpose';
import type { ProjectContext } from '../../../../../packages/commands/generated/typescript/ProjectContext';
import type { WorkspaceSnapshot } from '../../services/projectHost';
import { dispatch, nativeHostAvailable } from '../../services/projectHost';
import {
  agentCodexConfiguration, authorizeAgent, grantAgentTask, readAgentConnections, revokeAgent,
  selectAgentDirectory, updateAgentAuthority, type AgentAuthorityChoice, type AgentConnectionInfo,
  type AgentGrantChoice,
} from '../../services/agentConnection';
import './agent.css';

export interface AgentEditorState { identity: string; dirty: boolean; busy: boolean }
export const agentEditorIdentity = (workspace: WorkspaceSnapshot) =>
  JSON.stringify([workspace.context.host.session_id, workspace.project?.id ?? null, 'agent']);
export interface AgentWorkspaceProps {
  workspace: WorkspaceSnapshot;
  onRefresh: () => Promise<unknown>;
  onClose: () => void;
  onEditorStateChanged?: (state: AgentEditorState) => void;
}
const readOnlyOperations: OperationName[] = ['describe_capabilities', 'get_context', 'get_object',
  'query_module_library', 'get_run', 'get_task'];
const label: Partial<Record<OperationName, string>> = {
  describe_capabilities: '能力发现', get_context: '当前工程', get_object: '读取对象',
  query_module_library: '模块与模板', get_run: '运行结果', get_task: '任务状态',
  apply_commands: '分批编辑', check_factory_draft: '工程与场地检查', render_view: '真实画面反馈',
  run_scenario: '运行彩排', compare_runs: '方案比较', compute_packaging: '纸包装计算',
  cancel_task: '任务控制', adopt_artifact: '采用专业成果', import_package: '导入',
  save_project: '保存工程', export_package: '导出模块包', export_evaluation: '四格式评估',
};
const pretty = (value: unknown) => JSON.stringify(value, null, 2);

export function AgentWorkspace({ workspace, onRefresh, onClose, onEditorStateChanged }: AgentWorkspaceProps) {
  const identity = agentEditorIdentity(workspace);
  const [info, setInfo] = useState<AgentConnectionInfo | null>(null);
  const [observedContext, setObservedContext] = useState<ProjectContext | null>(null);
  const [session, setSession] = useState('');
  const available = workspace.capabilities.implemented_operations.map(value => value.operation);
  const [operations, setOperations] = useState<OperationName[]>(() => readOnlyOperations.filter(value => available.includes(value)));
  const [drafts, setDrafts] = useState<string[]>([]);
  const [viewControl, setViewControl] = useState(false);
  const [grants, setGrants] = useState('[]');
  const [goal, setGoal] = useState('');
  const [bridgePath, setBridgePath] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [directory, setDirectory] = useState<unknown>(null);
  const [purpose, setPurpose] = useState<FilePurpose>('save_project');
  const [taskId, setTaskId] = useState('');
  const [taskControls, setTaskControls] = useState(false);
  const [closing, setClosing] = useState(false);
  const defaultForm = useMemo(() => pretty({ operations: readOnlyOperations.filter(value => available.includes(value)),
    drafts: [], viewControl: false, grants: '[]' }), [available.join('|')]);
  const [acceptedForm, setAcceptedForm] = useState(defaultForm);
  const form = pretty({ operations, drafts, viewControl, grants });
  const dirty = form !== acceptedForm || goal !== '' || bridgePath !== '';
  const current = useRef(identity); current.current = identity;
  const refreshCurrent = useRef(onRefresh); refreshCurrent.current = onRefresh;
  const displayedRevision = useRef(workspace.project?.revision); displayedRevision.current = workspace.project?.revision;
  const observer = useRef(onEditorStateChanged); observer.current = onEditorStateChanged;
  useEffect(() => { observer.current?.({ identity, dirty, busy }); }, [identity, dirty, busy]);
  useEffect(() => {
    current.current = identity;
    return () => { if (current.current === identity) current.current = ''; observer.current?.({ identity, dirty: false, busy: false }); };
  }, [identity]);
  const authorized = info?.connections.find(value => value.session_id === session);
  const definitionScopes = [...new Set(workspace.capabilities.implemented_operations.flatMap(value => value.allowed_scopes)
    .flatMap(scope => scope.kind === 'definition_draft' ? [scope.data] : []))];

  useEffect(() => {
    if (!nativeHostAvailable()) return;
    let active = true;
    let reading = false;
    let observedKey: string | null = null;
    let timer: ReturnType<typeof setInterval> | undefined;
    const read = async () => {
      if (reading) return;
      reading = true;
      try {
        const value = await readAgentConnections();
        if (!active) return;
        setInfo(value);
        if (workspace.project && (observedKey !== null || value.connections.some(peer => peer.authorized && !peer.revoked))) {
          const context = await dispatch<ProjectContext>({ api_version: '0.1', request_id: `ui-agent-observe-${crypto.randomUUID()}`,
            project_id: workspace.project.id, scope: { kind: 'project' }, operation: 'get_context', params: { selection: [] } });
          if (!active) return;
          if (context.project?.id !== workspace.project.id) throw new Error('活动工程已改变；当前Agent输入保留，请重新核对连接范围。');
          setObservedContext(context);
          const key = pretty([context.project.revision, context.project.dirty, context.tasks.map(task => [task.id, task.state]), context.run_summaries?.map(run => [run.run_id, run.computation_status, run.computed_until_ms])]);
          const changed = observedKey !== null && observedKey !== key;
          observedKey = key;
          if (changed || context.project.revision !== displayedRevision.current) await refreshCurrent.current();
        }
      }
      catch (failure) { if (active) { setError(String(failure)); clearInterval(timer); } }
      finally { reading = false; }
    };
    void read(); timer = setInterval(() => { void read(); }, 1000);
    return () => { active = false; clearInterval(timer); };
  }, [identity]);

  async function action(work: (isCurrent: () => boolean) => Promise<void>) {
    const origin = identity;
    const isCurrent = () => current.current === origin;
    setBusy(true); setError(''); setNotice('');
    try { await work(isCurrent); }
    catch (failure) { if (isCurrent()) setError(failure instanceof Error ? failure.message : String(failure)); }
    finally { if (isCurrent()) setBusy(false); }
  }
  function choice(): AgentAuthorityChoice {
    const edit: AgentGrantChoice[] = JSON.parse(grants);
    if (!Array.isArray(edit)) throw new Error('编辑权限应为数组；输入已保留。');
    return { project_id: workspace.project?.id ?? null, operations, definition_drafts: drafts,
      allow_view_control: viewControl, edit };
  }
  const configuration = info && bridgePath.trim() ? agentCodexConfiguration(bridgePath.trim(), info) : '';
  const prompt = `${goal.trim()}\n\n先调用 superfactory_describe_capabilities 和 superfactory_get_context，读取 integrations/agent/skills/superfactory-workflow/SKILL.md。只使用真实可用工具与本次已授权工程、对象和字段。目标不是产量结果；缺少模板、班次、工时或尺寸时保留草稿并集中说明，不编造参数。分批编辑后，基于同一修订检查布局、取得真实画面并运行；初案实际达标即单Run保存及四格式交付。不足才在已授权变量内最多修正三轮；无有效变化停止。所有产物可由人继续编辑。`;

  function addGrantExample() {
    if (!workspace.project) return;
    const proposal: AgentGrantChoice[] = [{ scope: { kind: 'edit', data: { kind: 'project', data: workspace.project.id } },
      branches: ['set_project_settings'], objects: [{ target: { kind: 'project', id: workspace.project.id },
        fields: [{ pointer: '/title', element_id: null }], can_remove: false }], creates: [], source_versions: [], reusable_resources: [] }];
    setGrants(pretty(proposal));
  }

  return <section className="agent-workspace" aria-label="外部Agent工作台">
    <header><div><h2>外部 Agent · 同一工程</h2><p>连接 → 确认范围 → 分批搭建 → 检查与画面反馈 → 彩排 → 评估交付</p></div>
      <button disabled={busy} onClick={() => dirty ? setClosing(true) : onClose()}>关闭工作台</button></header>
    {!nativeHostAvailable() ? <p role="alert">请在桌面应用中连接；网页预览没有替代 Host 或模拟 Agent。</p> : <>
      <div className="agent-columns">
        <div>
          <fieldset disabled={busy}><legend>1 · 连接现有桌面 Host</legend>
            <p>Host：{info?.host_instance_id ?? '正在读取'} · 工程：{workspace.project?.title ?? '尚无活动工程'}</p>
            <label>实际桥程序完整路径<input value={bridgePath} onChange={event => setBridgePath(event.target.value)} placeholder="实际发布包中的 superfactory-mcp-bridge.exe" /></label>
            <p>以下配置仅供复制到你选择的客户端；软件不改全局配置、不启动第二个工程引擎。</p>
            <textarea readOnly aria-label="Codex MCP连接配置" value={configuration} rows={6} />
            <button disabled={!configuration} onClick={() => void action(async valid => { await navigator.clipboard.writeText(configuration); if (valid()) setNotice('已复制当前 Host 的临时连接配置。Host 重启后需重新读取。'); })}>复制连接配置</button>
            <button onClick={() => void action(async valid => { const next = await readAgentConnections(); if (valid()) setInfo(next); })}>重新读取连接</button>
            <label>Windows 已验证的连接<select value={session} onChange={event => { setSession(event.target.value); setDirectory(null); }}>
              <option value="">明确选择连接</option>{info?.connections.map(value => <option key={value.session_id} value={value.session_id}>{value.session_id} · PID {value.pid} · 登录会话 {value.os_session} · {value.revoked ? '已撤销' : value.authorized ? '已授权' : '等待确认'}</option>)}
            </select></label>
            {!info?.connections.length && <p>尚未检测到客户端。启动真实客户端后，在此确认其连接；地址本身不授予权限。</p>}
          </fieldset>
          <fieldset disabled={busy || !authorized || authorized.revoked}><legend>2 · 明确本次允许的范围</legend>
            <p>下列为待提交选择，默认只读。授权绑定工程 {workspace.project?.id ?? '无'}；切换工程后必须重新核对。</p>
            <div className="agent-operations">{available.map(operation => <label key={operation}><input type="checkbox" checked={operations.includes(operation)} onChange={() => setOperations(values => values.includes(operation) ? values.filter(value => value !== operation) : [...values, operation])} />{label[operation] ?? operation}<small>{operation}</small></label>)}</div>
            {!!definitionScopes.length && <details><summary>可授权的真实定义草稿</summary>{definitionScopes.map(id => <label key={id}><input type="checkbox" checked={drafts.includes(id)} onChange={() => setDrafts(values => values.includes(id) ? values.filter(value => value !== id) : [...values, id])} />{id}</label>)}</details>}
            <label><input type="checkbox" checked={viewControl} onChange={event => setViewControl(event.target.checked)} />允许主动切换主窗口视图；未选中时仅捕获独立反馈画面</label>
            <details><summary>对象、字段、新增数量与复用资源权限</summary>
              <p>按客户端提出的具体清单核对 Scope、命令、JSON Pointer、新增上限、精确模块版本与资源ID；不能用整个对象提交绕过字段限制。空数组不允许编辑。</p>
              <textarea aria-label="编辑权限JSON" value={grants} rows={12} onChange={event => setGrants(event.target.value)} />
              <button disabled={!workspace.project} onClick={addGrantExample}>填入仅修改当前工程标题的权限示例</button>
            </details>
            <button onClick={() => void action(async valid => { if (!authorized) throw new Error('请选择当前实际连接。'); const next = choice(); if (authorized.authorized) await updateAgentAuthority(session, next); else await authorizeAgent(session, next); if (!valid()) return; const nextInfo = await readAgentConnections(); if (!valid()) return; setAcceptedForm(form); setInfo(nextInfo); setNotice('所选范围已提交给当前 Native Host；后续请求仍由 Host 逐项检查。'); })}>{authorized?.authorized ? '提交新的授权范围' : '确认并授权此连接'}</button>
            <button disabled={!authorized?.authorized} onClick={() => void action(async valid => { await revokeAgent(session); if (!valid()) return; setInfo(await readAgentConnections()); setDirectory(null); setNotice('连接已撤销；已接收的 Host 任务仍由同一 Host 管理，不自动重放。'); })}>撤销此连接</button>
          </fieldset>
        </div>
        <div>
          <fieldset disabled={busy}><legend>3 · 目标与真实反馈</legend>
            <label>用一句话说明产品、目标和场地<textarea value={goal} onChange={event => setGoal(event.target.value)} rows={4} placeholder="例如：在给定场地内规划纸箱工厂；说明日产目标、长宽高及已有设备。缺少的班次和工时仍需补齐。" /></label>
            <button disabled={!goal.trim()} onClick={() => void action(async valid => { await navigator.clipboard.writeText(prompt); if (valid()) setNotice('已复制业务目标与工作流约束；请交给已连接的真实 Agent 执行。'); })}>复制给外部 Agent 的任务</button>
            <p>本面板不把文本粘贴伪装成 Agent 运行。Agent 实际调用共享编辑命令后，可在画布继续编辑；检查结果、Run 和报告都来自 Host。</p>
            <button onClick={() => void action(async valid => { await onRefresh(); if (valid()) setNotice('已请求刷新同一工程的真实修订、任务与运行结果。'); })}>刷新 Agent 已提交的工程变化</button>
            <p>当前工程修订：{workspace.project?.revision ?? '—'}。已授权连接的真实修订、任务和Run变化会自动刷新，保留本面板输入。初案达标即可交付；比较与修正仅用于真实不足案例。</p>
          </fieldset>
          <fieldset disabled={busy || !authorized?.authorized || authorized.revoked}><legend>4 · 目录与已存在任务</legend>
            <label>为此连接选择目录用途<select value={purpose} onChange={event => setPurpose(event.target.value as FilePurpose)}>
              <option value="save_project">保存可编辑工程</option><option value="export_evaluation">导出评估文档</option><option value="export_execution_report">导出执行报告</option><option value="export_cloud_data">导出全部固定筛选数据</option><option value="export_package">导出模块包</option><option value="import_package">导入工程目录</option>
            </select></label>
            <button onClick={() => void action(async valid => { const selected = await selectAgentDirectory(session, purpose); if (valid()) { setDirectory(selected); setNotice('目录令牌仅授予此实际连接及所选用途；可复制给该 Agent。'); } })}>在 Native 对话框选择目录</button>
            {directory !== null && <><pre>{pretty(directory)}</pre><button onClick={() => void action(async valid => { await navigator.clipboard.writeText(pretty(directory)); if (valid()) setNotice('已复制本连接的目录令牌。'); })}>复制目录令牌</button></>}
            <label>将本 Host 已知任务授权给当前连接<select value={taskId} onChange={event => setTaskId(event.target.value)}><option value="">选择实际 TaskId</option>{(observedContext?.tasks ?? workspace.context.tasks).map(task => <option key={task.id} value={task.id}>{task.id} · {task.state}</option>)}</select></label>
            <label><input type="checkbox" checked={taskControls} onChange={event => setTaskControls(event.target.checked)} />同时允许该任务实际支持的控制</label>
            <button disabled={!taskId} onClick={() => void action(async valid => { await grantAgentTask(session, taskId, taskControls); if (valid()) setNotice('已授权读取所选实际任务；不会重新执行任务或恢复旧文件权限。'); })}>确认任务授权</button>
          </fieldset>
          <details><summary>如何交付与人工接手</summary><p>保存可编辑工程并导出 PDF、HTML、CSV、JSON。缺项保留草稿；不将目标数、动画或假设当作已完成产量。生成定义、提交版本、采用模块、运行和导出是独立命令。当前 Run 的冻结输入保持不变，后续编辑进入下一方案。</p></details>
        </div>
      </div>
    </>}
    {error && <p role="alert" className="agent-error">{error}</p>}{notice && <p role="status">{notice}</p>}
    {closing && <div className="modal-backdrop"><section className="modal" role="alertdialog" aria-modal="true" aria-label="未提交Agent输入"><h2>未提交的 Agent 输入</h2><p>连接路径、任务说明或授权选择仍有未提交更改。已授予的连接权限不会随关闭工作台自动撤销。</p><footer><button autoFocus onClick={() => setClosing(false)}>保留输入，返回</button><button onClick={() => { observer.current?.({ identity, dirty: false, busy: false }); onClose(); }}>丢弃本地输入并关闭</button></footer></section></div>}
  </section>;
}
