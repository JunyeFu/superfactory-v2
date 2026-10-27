import { useMemo, useState } from 'react';
import type { BusinessEditorProps } from './types';
import { frozenProjectMatches, observedItems, quantityText, reasonNames } from '../timeline/projection';
import { timeText } from '../timeline/time';
export function BusinessRuntime(props:BusinessEditorProps){
  const [filter,setFilter]=useState(''),run=props.run;
  const items=useMemo(()=>observedItems(props.observation,run??null,props.atMs??null),[props.observation,run,props.atMs]);
  if(!run)return <><h2>实际接受与等待</h2><p>还没有实际Run。保存输入后明确重跑，计划值不画成实际产量。</p></>;
  const match=(id:string)=>id.toLowerCase().includes(filter.toLowerCase());
  const locate=(kind:Parameters<BusinessEditorProps['onLocate']>[0]['object']['kind'],id:string,at=props.atMs??run.computed_until_ms??run.window.start_ms)=>props.onLocate({object:{kind,id},source:'run',run_id:run.id,at_ms:at});
  return <><h2>实际接受与等待 · 只读</h2><p>Run {run.id} · 来源 {run.evidence_origin} · 冻结工程 r{run.input_ref.project_revision} / 方案 r{run.input_ref.scenario_revision}</p><p>观察窗 {timeText(run.window.start_ms)}—{timeText(run.window.end_ms)}；已计算至 {run.computed_until_ms==null?'未知':timeText(run.computed_until_ms)}。未完成不记作0秒。</p>
    {!frozenProjectMatches(props.frozenProject??null,run)&&<p role="alert">未提供匹配的冻结工程；下列结果保留原ID，不用当前草稿重解释历史。</p>}
    <p>计算状态 {run.computation_status} · 接受目标 {run.goal_met===true?'已达到':run.goal_met===false?'尚未达到':'未评价'}；计算结束不等于订单履行完成。</p>
    <input aria-label="查找实际身份" placeholder="订单/作业/物品/资源ID" value={filter} onChange={e=>setFilter(e.target.value)}/>
    <h3>整窗实际订单结果（区别于当前时点库存）</h3><table><thead><tr><th>订单</th><th>需求/接受/未满足</th><th>完工/迟交</th></tr></thead><tbody>{run.orders.filter(o=>match(o.order_id)).map(o=><tr key={o.order_id}><td><button onClick={()=>locate('order',o.order_id)}>{o.order_id}</button></td><td>{quantityText(o.demand)} / {quantityText(o.accepted)} / {quantityText(o.outstanding)}</td><td>{o.completion_ms==null?'未完成':timeText(o.completion_ms)} / {o.tardiness_ms==null?'未知':timeText(o.tardiness_ms)}</td></tr>)}</tbody></table>
    <h3>等待条件与相关对象</h3>{run.waits.filter(w=>match(w.object_ref.id)||w.reasons.some(r=>match(r.blocking_object?.id??''))).slice(0,100).map((w,i)=><article className="sf-business-readonly" key={i}><button onClick={()=>locate(w.object_ref.kind,w.object_ref.id,w.interval.start_ms)}>{w.object_ref.id} · {timeText(w.interval.start_ms)}—{timeText(w.interval.end_ms)}</button><p>历史阶段：{w.phase_ref??'未绑定'} · 结束原因 {w.end_cause}</p>{w.reasons.map((reason,j)=><p key={j}>{reasonNames[reason.kind]}：{reason.required} · 下一已知事件 {reason.next_known_boundary_ms==null?'未安排/未知':timeText(reason.next_known_boundary_ms)} {reason.blocking_object&&<button onClick={()=>locate(reason.blocking_object!.kind,reason.blocking_object!.id,w.interval.start_ms)}>定位 {reason.blocking_object.id}</button>}</p>)}</article>)}
    <h3>真实接受事件</h3>{run.acceptance_events.map((event,i)=><p key={i}>{timeText(event.at_ms)} · {quantityText(event.quantity)} · {event.acceptance_stage} · <button onClick={()=>locate('location',event.destination,event.at_ms)}>{event.destination}</button> · {event.order_id??'无订单接受目标'}</p>)}
    <h3>观察时点物品</h3><p>{props.atMs==null?'未选择实际时点':timeText(props.atMs)} · {items===null?'完整同Run时点数据未齐，不使用末态库存填补':`${items.length}条真实身份记录；输入变化不会修改这些物品`}</p>{items?.filter(i=>match(i.id)||match(i.holder.id)).slice(0,100).map(i=><p key={i.id}><button onClick={()=>locate('item',i.id)}>{i.id}</button> · {quantityText(i.quantity)} · {i.lifecycle} / {i.quality} · {i.holder.kind} {i.holder.id}</p>)}
    <h3>真实资源持有</h3>{run.resource_claims.filter(c=>match(c.resource_id)||match(c.job_id)).slice(0,100).map(c=><p key={c.id}><button onClick={()=>locate('resource',c.resource_id,c.intervals[0]?.start_ms)}>{c.resource_id}</button> · {c.units}容量 · 作业{c.job_id} · {c.phase_ref??'全作业'} · {c.intervals.map((i,index)=>`${timeText(i.start_ms)}—${timeText(i.end_ms)} (${c.interval_end_causes[index]})`).join('；')}</p>)}
    <h3>来源与待补</h3>{run.limitations.map((issue,i)=><p key={i}>{issue.message}{issue.object_ref&&<button onClick={()=>locate(issue.object_ref!.kind,issue.object_ref!.id)}>定位缺项</button>}</p>)}<p>成本输入缺项保持未知，不能据时间结果补出价格。完整评估从同源报告面板读取。</p>
    <p>身份、等待和资源列表每次显示最多100项；按完整ID筛选可定位其余记录，未截断原Run。</p>
  </>;
}
