import { useEffect, useRef, useState } from 'react';
import type { ModuleCandidate } from '../../../../../packages/commands/generated/typescript/ModuleCandidate';
import type { ModuleTemplateCandidates } from '../../../../../packages/commands/generated/typescript/ModuleTemplateCandidates';
import type { ApiCategory } from '../../../../../packages/commands/generated/typescript/ApiCategory';
import type { LibrarySourceScope } from '../../../../../packages/commands/generated/typescript/LibrarySourceScope';
import type { ModuleRef } from '../../../../../packages/commands/generated/typescript/ModuleRef';
import type { ProjectState } from '../../services/projectHost';
import { moduleKey, readModuleLibrary } from '../../services/projectHost';
import type { TemplatePreview } from '../../../../../packages/commands/generated/typescript/TemplatePreview';
import { TemplatePreparation } from './TemplatePreparation';

interface Props {
  project: ProjectState | null;
  busy: boolean;
  initialModule?: ModuleRef;
  initialCategory?: ApiCategory;
  onDirtyChange?(dirty: boolean): void;
  onClose: () => void;
  onAdopt: (base: ProjectState, ref: ModuleRef) => Promise<void>;
  onTemplate: (base: ProjectState, preview: TemplatePreview) => Promise<void>;
}
const categories: [ApiCategory, string][] = [['items', '物品'], ['devices', '设备'], ['resources', '资源'], ['tasks', '任务'], ['operations', '工序'], ['transport', '运输'], ['templates', '模板'], ['professional', '专业'], ['media', '表现']];
const sourceLabel: Record<LibrarySourceScope, string> = { bundled: '软件内置库', project_packages: '当前工程包', user_granted_libraries: '已授权目录' };

export function ModuleLibrary({ project, busy, initialModule, initialCategory, onDirtyChange, onClose, onAdopt, onTemplate }: Props) {
  const [base] = useState(project);
  const [category, setCategory] = useState<ApiCategory | ''>(initialCategory ?? '');
  const [sources, setSources] = useState<LibrarySourceScope[]>(['bundled', 'project_packages']);
  const [result, setResult] = useState<ModuleTemplateCandidates | null>(null);
  const [selected, setSelected] = useState<ModuleCandidate | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [templateDirty, setTemplateDirty] = useState(false), [leaving, setLeaving] = useState(false);
  useEffect(() => { onDirtyChange?.(templateDirty); }, [templateDirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);
  const epoch = useRef(0);
  useEffect(() => {
    const request = ++epoch.current;
    setLoading(true); setError(''); setSelected(null);
    void readModuleLibrary(base?.id ?? null, category || null, sources).then(value => {
      if (request === epoch.current) {
        setResult(value);
        if (initialModule) setSelected(value.candidates.find(candidate => moduleKey(candidate.module_ref) === moduleKey(initialModule)) ?? null);
      }
    }).catch(failure => {
      if (request === epoch.current) { setResult(null); setError(failure instanceof Error ? failure.message : String(failure)); }
    }).finally(() => { if (request === epoch.current) setLoading(false); });
    return () => { epoch.current++; };
  }, [base?.id, category, sources]);
  const adopted = selected && base?.module_refs.some(ref => moduleKey(ref) === moduleKey(selected.module_ref));
  async function adopt() {
    if (!base || !selected) return;
    setError('');
    try { await onAdopt(base, selected.module_ref); }
    catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); }
  }
  return <section className="module-library" aria-label="模块与模板库">
    <header><div><small>{base ? `工程 r${base.revision} · 确认采纳后形成新修订` : '尚未创建工程 · 可浏览实际目录'}</small><h2>模块与模板库</h2></div><button disabled={busy} onClick={() => templateDirty ? setLeaving(true) : onClose()}>返回画布</button></header>
    <div className="library-filters"><label>类别<select disabled={templateDirty} value={category} onChange={event => setCategory(event.target.value as ApiCategory | '')}><option value="">全部九类</option>{categories.map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label><fieldset><legend>来源</legend>{(Object.keys(sourceLabel) as LibrarySourceScope[]).map(source => <label key={source}><input type="checkbox" disabled={templateDirty} checked={sources.includes(source)} onChange={event => setSources(previous => event.target.checked ? [...previous, source] : previous.filter(value => value !== source))} />{sourceLabel[source]}</label>)}</fieldset></div>
    {error && <p role="alert" className="error-message">{error}</p>}
    {result?.missing.map((issue, index) => <p className="issue-row" key={index}>{issue.message}</p>)}
    <div className="library-body"><div className="library-list" aria-label="真实模块候选">{loading ? <p>读取实际模块目录…</p> : result?.candidates.map(candidate => <button key={moduleKey(candidate.module_ref)} disabled={templateDirty && selected !== candidate} className={selected === candidate ? 'selected' : ''} onClick={() => setSelected(candidate)}><strong>{candidate.title}</strong><span>{categories.find(([key]) => key === candidate.category)?.[1]} · {candidate.module_ref.version}</span><small>{candidate.module_ref.id}</small></button>)}{!loading && result?.candidates.length === 0 && <p className="muted">所选来源与类别没有返回候选。</p>}</div><div className="library-details">{selected ? <><h3>{selected.title}</h3><dl><dt>精确标识</dt><dd>{selected.module_ref.id} @ {selected.module_ref.version}</dd><dt>适用性</dt><dd>{selected.applicable ? '目录未报告阻断；实际采纳仍由 Host 重验' : '存在待核条件'}</dd><dt>工程选择</dt><dd>{adopted ? '此精确版本已在工程中' : '尚未采纳此精确版本'}</dd></dl>{selected.sources.map(source => <p key={source.id}>{source.title} · {source.locator ?? '未提供定位'}</p>)}{selected.missing.map((issue, index) => <p className="issue-row" key={index}>{issue.message}</p>)}<p>采纳定义不会创建设备、物品库存或方案。设备须在画布中明确创建与绑定；模板布局与示例输入另行确认。</p><button className={selected.category === 'templates' ? '' : 'primary'} disabled={!base || busy || loading || adopted === true || !selected.applicable} onClick={() => void adopt()}>只采纳定义到工程</button>{!base && <p className="muted">先创建工程，再确认采纳。</p>}{base && selected.category === 'templates' && <TemplatePreparation onDirtyChange={setTemplateDirty} key={moduleKey(selected.module_ref)} project={base} moduleRef={selected.module_ref} busy={busy} onApply={onTemplate} />}</> : <p className="muted">选择一个实际候选，核对精确版本与缺项。</p>}</div></div>
    {leaving && <section role="dialog" aria-label="放弃模板准备输入"><p>模板配置尚未采纳。请返回保留输入，或明确放弃后离开。</p><button onClick={() => setLeaving(false)}>返回保留模板输入</button><button onClick={onClose}>明确放弃模板输入</button></section>}
  </section>;
}
