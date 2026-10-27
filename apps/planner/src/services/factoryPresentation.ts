import { dispatch } from './projectHost';
import { readMediaAsset, type MediaAssetReadRequest } from './mediaAssets';
import { ProcessSourceChanged, ProcessUnavailable, readActualPortOutputs, type PresentationReadPorts, type ProcessScope } from './presentationHost';
import type { ApiRequest } from '../../../../packages/commands/generated/typescript/ApiRequest';
import type { DomainObject } from '../../../../packages/commands/generated/typescript/DomainObject';
import type { ProjectState } from '../../../../packages/commands/generated/typescript/ProjectState';
import type { ModuleRef } from '../../../../packages/commands/generated/typescript/ModuleRef';
import type { ModulePackage } from '../../../../packages/commands/generated/typescript/ModulePackage';
import type { RunResult } from '../../../../packages/commands/generated/typescript/RunResult';
import type { RunView } from '../../../../packages/commands/generated/typescript/RunView';
import type { RunObjectsPage } from '../../../../packages/commands/generated/typescript/RunObjectsPage';
import type { OperationDefinition } from '../../../../packages/commands/generated/typescript/OperationDefinition';
import type { ActualMediaFrame, ActualPortOutputFrame } from '../../../../packages/presentation/src';
import type { MediaPresentationCommonProps } from '../features/media/types';
import type { RuntimeItem } from '../../../../packages/commands/generated/typescript/RuntimeItem';
import type { JobState } from '../../../../packages/commands/generated/typescript/JobState';
import type { ObjectRef } from '../../../../packages/commands/generated/typescript/ObjectRef';
import type { ItemDisplaySlot } from '../../../../packages/commands/generated/typescript/ItemDisplaySlot';
import type { ResourceInstance } from '../../../../packages/commands/generated/typescript/ResourceInstance';
import type { ItemSlotLayout } from '../../../../packages/commands/generated/typescript/ItemSlotLayout';
import type { RunObservationReadRequest } from './runObservation';

export interface FactoryPresentationContext extends ProcessScope {
  project: ProjectState;
  view: { run_id: string; at_ms: number } | null;
  /** The workbench already obtained these exact frozen inputs for this Run. */
  preparedRun?: { run: RunResult; modules: ReadonlyMap<string, ModulePackage> };
}
export interface FactoryDeviceMedia {
  device_id: string;
  media: Omit<MediaPresentationCommonProps, 'lod'> | null;
  frame: ActualMediaFrame | null;
  unavailable: string | null;
  projection: 'active' | 'terminal' | 'static' | 'unavailable';
  port_outputs?: ActualPortOutputFrame;
}
/** A read-only projection of the canonical frozen owner layout, never a Host endpoint. */
export interface FactoryItemAttachment {
  owner_ref: ObjectRef;
  slot: ItemDisplaySlot;
}
export interface FactoryPresentationPorts extends PresentationReadPorts {
  readObservation?(request: RunObservationReadRequest): Promise<RunObjectsPage>;
}
export interface FactoryActualItem {
  item: RuntimeItem;
  /** Only the unique actual Job currently holding this item can supply a phase clock. */
  holder_frame?: ActualMediaFrame | null;
  anchor_ref: ObjectRef | null;
  attachment: FactoryItemAttachment | null;
  media: Omit<MediaPresentationCommonProps, 'lod'> | null;
  unavailable: string | null;
}
export interface FactoryResourceMedia {
  resource: ResourceInstance;
  worker_status?: { kind: 'unobserved' | 'available' | 'claimed'; job_ids: string[] };
  media: FactoryActualItem['media'];
  unavailable: string | null;
}
/** A known declared home remains visible while the next actual observation is pending. */
export function declaredFactoryWorkers(project: ProjectState | null): FactoryResourceMedia[] {
  return (project?.resources ?? []).filter(resource => resource.kind === 'worker' && resource.home_location
    && project!.locations.some(location => location.id === resource.home_location && location.hall_id))
    .map(resource => ({ resource, media: null, unavailable: null, worker_status: { kind: 'unobserved', job_ids: [] } }));
}
/** Declared home position and actual JobsAt claims, never a walking trajectory. */
export function factoryWorkerLabel(input: FactoryResourceMedia): string {
  const { resource, worker_status: status } = input;
  const occupation = status!.kind === 'claimed' ? `占用作业 ${status!.job_ids.join('、')}`
    : status!.kind === 'available' ? '本时点未占用' : '占用尚未读取';
  return `${resource.title} · ${resource.id} · 人员角色 ${resource.role} · 声明位置 ${resource.home_location} · ${occupation}`;
}
export interface FactoryPresentationSource {
  key: string;
  project: ProjectState;
  run: RunResult | null;
  observation: RunObjectsPage | null;
  devices: FactoryDeviceMedia[];
  /** Already read exact definitions, including frozen versions when inspecting a Run. */
  modules?: ReadonlyMap<string, ModulePackage>;
  resources?: FactoryResourceMedia[];
  items: FactoryActualItem[];
  raw: { job_pages: RunObjectsPage[]; item_pages: RunObjectsPage[]; frozen_task?: string };
}
export const factoryContextKey = (c: FactoryPresentationContext) => JSON.stringify([
  c.session_id, c.project_id, c.scope, c.scope_revision, c.project.id, c.project.revision, c.view,
]);
const sameRef = (a: ModuleRef, b: ModuleRef) => a.id === b.id && a.version === b.version;
export type FactoryHolderJob = Pick<JobState, 'id' | 'execution_ref'>;
const jobCatalogs = new WeakMap<readonly FactoryHolderJob[], Map<string, FactoryHolderJob[]>>();
const itemCatalogs = new WeakMap<RuntimeItem[], Map<string, RuntimeItem>>();
function jobsById(jobs: readonly FactoryHolderJob[]) {
  let index = jobCatalogs.get(jobs);
  if (!index) { index = new Map(); for (const job of jobs) { const rows = index.get(job.id) ?? []; rows.push(job); index.set(job.id, rows); } jobCatalogs.set(jobs, index); }
  return index;
}
function itemsById(items: RuntimeItem[]) {
  let index = itemCatalogs.get(items);
  if (!index) { index = new Map(); for (const item of items) if (!index.has(item.id)) index.set(item.id, item); itemCatalogs.set(items, index); }
  return index;
}
export function factoryItemHolder(item: RuntimeItem, items: RuntimeItem[], jobs: readonly FactoryHolderJob[]): ObjectRef | null {
  let current: RuntimeItem | undefined = item;
  const seen = new Set<string>();
  while (current && !seen.has(current.id)) {
    seen.add(current.id);
    if (current.holder.kind === 'location') return { kind: 'location', id: current.holder.id };
    if (current.holder.kind === 'transport') return null;
    if (current.holder.kind === 'job') {
      const found = jobsById(jobs).get(current.holder.id) ?? [];
      return found.length === 1 && found[0].execution_ref.kind === 'project_device'
        ? { kind: 'device', id: found[0].execution_ref.data.device_id } : null;
    }
    const parent = itemsById(items).get(current.holder.id);
    if (!parent?.contents.includes(current.id)) return null;
    current = parent;
  }
  return null;
}
export interface FactoryItemSlotProjection { attachment: FactoryItemAttachment | null; unavailable: string | null }
/** Deterministic snapshot layout from the same complete actual identity population. */
export function projectFactoryItemSlots(project: ProjectState, items: RuntimeItem[], jobs: readonly FactoryHolderJob[]): Map<string, FactoryItemSlotProjection> {
  const result = new Map<string, FactoryItemSlotProjection>();
  const groups = new Map<string, { owner: ObjectRef; layout: ItemSlotLayout | null | undefined; items: RuntimeItem[] }>();
  const itemType = (item: RuntimeItem) => project.item_types.find(type => type.id === item.item_type_ref.id && type.revision === item.item_type_ref.revision);
  for (const item of items) {
    const fail = (message: string) => result.set(item.id, { attachment: null, unavailable: message });
    if (['transformed','scrapped'].includes(item.lifecycle)) { fail('实际物品已转化或报废；保留身份，不复制活动库存几何。'); continue; }
    let owner: ObjectRef, layout: ItemSlotLayout | null | undefined;
    if (item.holder.kind === 'transport') { fail('unavailable · transport没有真实当前Pose；不猜路径或运输位置。'); continue; }
    if (item.holder.kind === 'job') {
      const found = jobsById(jobs).get(item.holder.id) ?? [];
      if (found.length !== 1 || found[0].execution_ref.kind !== 'project_device') { fail('unavailable · 当前holder没有唯一实际JobsAt设备映射。'); continue; }
      const execution = found[0].execution_ref;
      if (execution.kind !== 'project_device') { fail('unavailable · 实际holder不是设备执行。'); continue; }
      const device = project.devices.find(device => device.id === execution.data.device_id);
      if (!device) { fail('unavailable · 实际holder设备不在冻结Project。'); continue; }
      owner = { kind: 'device', id: device.id }; layout = device.item_layout;
    } else if (item.holder.kind === 'location') {
      const location = project.locations.find(location => location.id === item.holder.id);
      if (!location?.hall_id || !project.halls.some(hall => hall.id === location.hall_id)) { fail('unavailable · 实际Location缺少冻结厂房/位置来源。'); continue; }
      owner = { kind: 'location', id: location.id }; layout = location.item_layout;
    } else {
      const parent = itemsById(items).get(item.holder.id);
      if (!parent?.contents.includes(item.id)) { fail('unavailable · container缺少双向contents/holder；composition不是contents。'); continue; }
      const seen = new Set([item.id]); let cursor: RuntimeItem | undefined = parent, issue: string | null = null;
      while (cursor) {
        if (seen.has(cursor.id)) { issue = '容器父链存在循环'; break; } seen.add(cursor.id);
        if (['transformed','scrapped'].includes(cursor.lifecycle)) { issue = '父容器已转化或报废'; break; }
        if (cursor.holder.kind === 'transport') { issue = '父容器transport没有真实Pose'; break; }
        if (cursor.holder.kind !== 'container') { if (!factoryItemHolder(cursor, items, jobs)) issue = '父容器缺少实际空间holder'; break; }
        const next = itemsById(items).get(cursor.holder.id);
        if (!next?.contents.includes(cursor.id)) { issue = '父链contents/holder不互认'; break; } cursor = next;
      }
      if (issue) { fail(`unavailable · ${issue}。`); continue; }
      const type = itemType(parent);
      if (!type) { fail('unavailable · 父容器精确type@revision不在冻结Project。'); continue; }
      owner = { kind: 'item', id: parent.id }; layout = type.contents_layout;
    }
    const key = JSON.stringify([owner.kind, owner.id, item.item_type_ref.id, item.item_type_ref.revision]);
    const group = groups.get(key) ?? { owner, layout, items: [] }; group.items.push(item); groups.set(key, group);
  }
  for (const group of groups.values()) {
    const { owner, layout } = group;
    const ordered = [...group.items].sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
    const reference = ordered[0].item_type_ref;
    const slots = layout?.slots.filter(slot => slot.item_type_ref.id === reference.id && slot.item_type_ref.revision === reference.revision) ?? [];
    const groupIssue = !layout ? '未声明持久化展示布局' : ordered.length > slots.length ? `${ordered.length} identities / ${slots.length} declared display slots；整组不绘制` : null;
    ordered.forEach((item, index) => {
      const slot = slots[index];
      const unavailable = groupIssue ?? (owner.kind === 'location' ? slot.anchor_role != null ? '库位展示槽anchor_role必须为null' : null : !slot.anchor_role ? '设备/容器展示槽缺少明确命名anchor_role' : null);
      result.set(item.id, { attachment: unavailable ? null : { owner_ref: owner, slot }, unavailable: unavailable ? `unavailable · ${unavailable}。` : null });
    });
  }
  return result;
}
type ReadOperation = 'get_object' | 'get_run';
type ReadRequest = { [O in ReadOperation]: { operation: O; params: Extract<ApiRequest, { operation: O }>['params'] } }[ReadOperation];
let portOutputModule: Promise<typeof import('./runPortOutput')> | undefined;

/** A read-only source for the one physical scene. Final Jobs supply IDs only. */
export function createFactoryPresentation(current: () => FactoryPresentationContext | null,
  ports: FactoryPresentationPorts = { dispatch, readAsset: readMediaAsset,
    readObservation: async request => (await import('./runObservation')).readRunObservation(request),
    readPortOutput: async request => (await (portOutputModule ??= import('./runPortOutput'))).readRunPortOutput(request) }) {
  async function read(context: FactoryPresentationContext): Promise<FactoryPresentationSource> {
    const key = factoryContextKey(context);
    const assertCurrent = () => { const next = current(); if (!next || factoryContextKey(next) !== key) throw new ProcessSourceChanged(); };
    const request = async <T>(input: ReadRequest): Promise<T> => {
      assertCurrent();
      const value = await ports.dispatch<T>({ api_version: '0.1', request_id: `factory-media-${crypto.randomUUID()}`,
        project_id: context.project_id, scope: context.scope, ...input });
      assertCurrent(); return value;
    };
    assertCurrent();
    if (context.project.id !== context.project_id) throw new ProcessUnavailable('主画布工程与授权来源不一致。');
    let project = context.project, run: RunResult | null = null, observation: RunObjectsPage | null = null;
    const job_pages: RunObjectsPage[] = [], item_pages: RunObjectsPage[] = [];
    if (context.view) {
      const view = context.view;
      const full: RunView = context.preparedRun ? { kind: 'full', data: context.preparedRun.run }
        : await request<RunView>({ operation: 'get_run', params: { run_id: view.run_id, query: { kind: 'full_result' } } });
      if (full.kind !== 'full' || full.data.id !== view.run_id || full.data.input_ref.project_id !== context.project_id) throw new ProcessUnavailable('主画布没有取得本工程的精确Run。');
      run = full.data;
      if (run.computed_until_ms == null || view.at_ms < run.window.start_ms || view.at_ms > run.computed_until_ms || view.at_ms > run.window.end_ms) throw new ProcessUnavailable('观看时点超出实际Run前沿，不补画实际阶段。');
      const pages = async (objects: { kind: 'job'; id: string }[], target: RunObjectsPage[]) => {
        let after_index: number | null = null;
        do {
          const page: RunView = await request<RunView>({ operation: 'get_run', params: { run_id: view.run_id,
            query: { kind: 'objects_at', data: { at_ms: view.at_ms, objects, page: { after_index, limit: 200 } } } } });
          if (page.kind !== 'objects' || page.data.run_id !== view.run_id || page.data.at_ms !== view.at_ms) throw new ProcessUnavailable('主画布对象页的Run或时点不匹配。');
          if (page.data.next_index !== null && page.data.next_index <= (after_index ?? -1)) throw new ProcessUnavailable('主画布实际分页没有前进。');
          target.push(page.data); after_index = page.data.next_index;
        } while (after_index !== null);
      };
      // Identity catalog only: states/progress/execution mappings below come exclusively from JobsAt.
      if (ports.readObservation) {
        const page = await ports.readObservation({ session_id: context.session_id, project_id: context.project_id,
          scope: context.scope, scope_revision: context.scope_revision, ...view });
        assertCurrent();
        if (page.run_id !== view.run_id || page.at_ms !== view.at_ms || page.next_index !== null) throw new ProcessUnavailable('主画布完整观察的Run、时点或完整性不匹配。');
        job_pages.push({ ...page, objects: page.objects.filter(object => object.kind === 'job') });
        item_pages.push({ ...page, objects: page.objects.filter(object => object.kind === 'item') });
      } else {
        const selectors = run.jobs.map(job => ({ kind: 'job' as const, id: job.id }));
        // Public protocol consumers retain bounded selectors and actual pagination.
        for (let start = 0; start < selectors.length; start += 200) await pages(selectors.slice(start, start + 200), job_pages);
        await pages([], item_pages);
      }
      if (!context.preparedRun) {
        const frozen = await request<DomainObject>({ operation: 'get_object', params: { object_id: { kind: 'project', id: context.project_id },
          source: { kind: 'run_snapshot', data: { run_id: view.run_id, at_ms: view.at_ms } }, node_index: null } });
        if (frozen.kind !== 'project') throw new ProcessUnavailable('主画布未取得Run冻结工程。');
        project = frozen.data;
      }
      if (project.id !== context.project_id || project.revision !== run.input_ref.project_revision) throw new ProcessUnavailable('主画布未取得Run冻结工程修订。');
      observation = { run_id: run.id, at_ms: view.at_ms, next_index: null, objects: [
        ...job_pages.flatMap(page => page.objects), ...item_pages.flatMap(page => page.objects),
        ...project.resources.map(data => ({ kind: 'resource' as const, data })),
      ] };
    }
    const modules = new Map<string, Promise<ModulePackage>>();
    const module = (ref: ModuleRef) => {
      const id = JSON.stringify(ref);
      let pending = modules.get(id);
      if (!pending) {
        pending = (async () => {
          const prepared = context.view && context.preparedRun?.modules.get(JSON.stringify([ref.id, ref.version]));
          if (prepared) {
            if (!sameRef(prepared.definition, ref)) throw new ProcessUnavailable('主画布精确模块版本不可用。');
            return prepared;
          }
          const source = context.view ? { kind: 'run_module_version' as const, data: { ...context.view, module_ref: ref } }
            : { kind: 'module_version' as const, data: { module_ref: ref } };
          const value = await request<DomainObject>({ operation: 'get_object', params: { object_id: { kind: 'module', id: ref.id }, source, node_index: null } });
          if (value.kind !== 'module' || !sameRef(value.data.definition, ref)) throw new ProcessUnavailable('主画布精确模块版本不可用。');
          return value.data;
        })();
        modules.set(id, pending);
      }
      return pending;
    };
    return projectFactoryPresentation(context, { project, run, observation,
      jobs: job_pages.flatMap(page => page.objects).flatMap(object => object.kind === 'job' ? [object.data] : []),
      items: item_pages.flatMap(page => page.objects).flatMap(object => object.kind === 'item' ? [object.data] : []),
      raw: { job_pages, item_pages } }, { module, readAsset: ports.readAsset, readPortOutput: ports.readPortOutput }, assertCurrent);
  }
  return { read };
}

export interface FactoryProjectionInput {
  project: ProjectState; run: RunResult | null; observation: RunObjectsPage | null;
  jobs: JobState[]; items: RuntimeItem[]; raw: FactoryPresentationSource['raw'];
  /** Exact execution holder mappings without inventing unavailable JobState fields. */
  holderJobs?: readonly FactoryHolderJob[];
}
export interface FactoryProjectionPorts extends Pick<PresentationReadPorts, 'readAsset' | 'readPortOutput'> {
  module(reference: ModuleRef): Promise<ModulePackage>;
}
/** Shared read-only presentation of already obtained actual inputs; no Host dispatcher. */
export async function projectFactoryPresentation(context: FactoryPresentationContext, input: FactoryProjectionInput,
  ports: FactoryProjectionPorts, assertCurrent: () => void): Promise<FactoryPresentationSource> {
    const key = factoryContextKey(context), { project, run, observation, jobs } = input;
    const loadedModules = new Map<string, ModulePackage>();
    // One authorized observation owns the bytes; identical instances share its pending read.
    const assetReads = new Map<string, Promise<ArrayBuffer>>();
    const module = async (reference: ModuleRef) => {
      const value = await ports.module(reference);
      loadedModules.set(JSON.stringify([reference.id, reference.version]), value);
      return value;
    };
    const boundMedia = (ref: ModuleRef, presentation: MediaPresentationCommonProps['presentation'], rights: MediaPresentationCommonProps['rights']): Omit<MediaPresentationCommonProps, 'lod'> => {
      const binding = { session_id: context.session_id, request: { project_id: context.project_id, scope: context.scope, scope_revision: context.scope_revision,
        source: context.view ? { kind: 'run_module_version' as const, data: { ...context.view, module_ref: ref } } : { kind: 'module_version' as const, data: { module_ref: ref } } } };
      const assertSource = (source: MediaAssetReadRequest['source']) => {
        assertCurrent();
        if (context.view ? source.kind !== 'run_module_version' || source.data.run_id !== context.view.run_id || source.data.at_ms !== context.view.at_ms : source.kind !== 'module_version') throw new ProcessUnavailable('媒体字节或依赖改变了已绑定Run/时点/来源。');
      };
      return { presentation, rights, binding, async readAsset(request) {
        assertSource(request.source);
        if (request.project_id !== context.project_id || JSON.stringify(request.scope) !== JSON.stringify(context.scope) || request.scope_revision !== context.scope_revision) throw new ProcessUnavailable('媒体字节改变了已绑定工程或Scope修订。');
        const key = JSON.stringify([request.source, request.asset]);
        let pending = assetReads.get(key);
        if (!pending) { pending = ports.readAsset(request); assetReads.set(key, pending); }
        const bytes = await pending; assertCurrent(); return bytes;
      }, async resolveModule(source) { assertSource(source); return module(source.data.module_ref); } };
    };
    const deviceJobs = new Map<string, JobState[]>();
    for (const job of jobs) if (job.execution_ref.kind === 'project_device') {
      const id = job.execution_ref.data.device_id, rows = deviceJobs.get(id) ?? [];
      rows.push(job); deviceJobs.set(id, rows);
    }
    const devices = await Promise.all(project.devices.map(async (device): Promise<FactoryDeviceMedia> => {
      let port_outputs: ActualPortOutputFrame | undefined;
      try {
        const pkg = await module(device.module_ref), payload = pkg.definition.payload;
        if (payload.category !== 'devices') throw new ProcessUnavailable('设备精确模块不是设备定义。');
        if (run && observation) port_outputs = await readActualPortOutputs(context, { run_id: run.id, at_ms: observation.at_ms, device, ports: payload.data.ports }, ports, assertCurrent);
        const presentation = device.presentation_override ?? payload.data.presentation;
        if (!presentation) throw new ProcessUnavailable('此设备未声明详细表现素材。');
        const mapped = deviceJobs.get(device.id) ?? [];
        const at = context.view?.at_ms;
        // JobsAt retains completed and queued executions. Only an actual open
        // execution or an explicit capacity holder currently owns this device.
        const active = mapped.filter(job => job.state !== 'completed' && job.state !== 'cancelled' && at != null
          && (job.finished_at_ms == null || at < job.finished_at_ms)
          && (job.started_at_ms != null && job.started_at_ms <= at || job.execution_ref.kind === 'project_device'
            && [...job.job_resources, ...job.phase_resources].includes(job.execution_ref.data.execution_capacity_ref.id)));
        if (active.length > 1) throw new ProcessUnavailable('同一时点有多个实际活动Job占用/执行此设备；无法唯一投影，未选择首个。');
        // A single historical execution has an unambiguous terminal pose.
        // Multiple completed histories with no owner stay static; no latest/first guess.
        const terminal = mapped.length === 1 && mapped[0].state === 'completed' && mapped[0].finished_at_ms != null
          && at != null && mapped[0].finished_at_ms <= at ? mapped[0] : undefined;
        const job = active.length === 1 ? active[0] : terminal;
        const projection = active.length === 1 ? 'active' : terminal ? 'terminal' : 'static';
        let frame: ActualMediaFrame | null = null;
        if (job && run && observation) {
          let operation: OperationDefinition;
          if (sameRef(job.operation_ref, device.module_ref) && payload.data.behavior?.selected === 'inline' && payload.data.behavior.inline) operation = payload.data.behavior.inline;
          else {
            const declared = payload.data.behavior.selected === 'operation_ref' ? payload.data.behavior.operation_ref : null;
            if (![declared, device.operation_binding, ...(device.route_operation_refs ?? [])].some(ref => ref != null && sameRef(ref, job.operation_ref))) throw new ProcessUnavailable('实际Job的operation不匹配冻结设备声明。');
            const operationPackage = await module(job.operation_ref);
            if (operationPackage.definition.payload.category !== 'operations') throw new ProcessUnavailable('实际operation精确模块类型不匹配。');
            operation = operationPackage.definition.payload.data;
          }
          frame = { run, snapshot: observation, job_id: job.id, operation };
        }
        if (frame) frame = { ...frame, port_outputs };
        return { device_id: device.id, media: boundMedia(device.module_ref, presentation, pkg.rights), frame, unavailable: null, projection, port_outputs };
      } catch (error) {
        assertCurrent();
        if (!(error instanceof ProcessUnavailable)) throw error;
        return { device_id: device.id, media: null, frame: null, unavailable: error.message, projection: 'unavailable', port_outputs };
      }
    }));
    const actualItems = input.items;
    const holderJobs = input.holderJobs ?? jobs;
    const attachments = projectFactoryItemSlots(project, actualItems, holderJobs);
    const items = await Promise.all(actualItems.map(async (item): Promise<FactoryActualItem> => {
      const anchor_ref = factoryItemHolder(item, actualItems, holderJobs);
      let media: FactoryActualItem['media'] = null;
      const unavailable = (reason: string): FactoryActualItem => ({ item, anchor_ref, attachment: null, media, unavailable: reason });
      const itemType = project.item_types.find(type => type.id === item.item_type_ref.id && type.revision === item.item_type_ref.revision);
      if (!itemType?.module_ref) return unavailable('unavailable · 冻结ItemType没有精确Items模块来源，不从名称/spec_ref/module_refs猜测。');
      try {
        const pkg = await module(itemType.module_ref), payload = pkg.definition.payload;
        if (payload.category !== 'items' || !payload.data.presentation_or_media_ref) return unavailable('精确Items模块没有物品媒体声明。');
        media = boundMedia(itemType.module_ref, payload.data.presentation_or_media_ref, pkg.rights);
      } catch (error) { assertCurrent(); if (!(error instanceof ProcessUnavailable)) throw error; return unavailable(error.message); }
      const projected = attachments.get(item.id)!;
      if (!projected.attachment) return unavailable(projected.unavailable!);
      const attachment = projected.attachment;
      const ownerFrame = anchor_ref?.kind === 'device' ? devices.find(d => d.device_id === anchor_ref.id)?.frame : null;
      const holder_frame = item.holder.kind === 'job' && ownerFrame?.job_id === item.holder.id ? ownerFrame : null;
      return { item, anchor_ref, attachment, media, holder_frame, unavailable: null };
    }));
    const resources = await Promise.all(project.resources.filter(resource => resource.kind === 'worker' && resource.home_location || resource.kind === 'reusable_carrier' && resource.presentation).map(async (resource): Promise<FactoryResourceMedia> => {
      if (!resource.home_location || !project.locations.some(location => location.id === resource.home_location && location.hall_id))
        return { resource, media: null, unavailable: '资源没有声明可用的厂房home_location。' };
      if (resource.kind === 'worker') {
        const held = jobs.filter(job => [...job.job_resources, ...job.phase_resources].includes(resource.id));
        return { resource, media: null, unavailable: null,
          worker_status: { kind: !observation ? 'unobserved' : held.length ? 'claimed' : 'available', job_ids: held.map(job => job.id) } };
      }
      const pkg = await module(resource.definition_ref);
      if (pkg.definition.payload.category !== 'resources') throw new ProcessUnavailable('资源精确模块不是Resources定义。');
      return { resource, media: boundMedia(resource.definition_ref, resource.presentation!, pkg.rights), unavailable: null };
    }));
    assertCurrent();
    return { key, project, run, observation, devices, modules: loadedModules, resources, items, raw: input.raw };
}
