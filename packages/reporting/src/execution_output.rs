//! Four-format byte production from one canonical ExecutionReport. Host owns
//! authorization, real output directories, Task cancellation and atomic commit.
use crate::comparison::{issue, value};
use crate::evaluation_output::{
    EvaluationFile, EvaluationOutputFormat, EvaluationResources, cancelled, escape,
};
use crate::execution_evaluation::check_export_content;
use printpdf::*;
use serde_json::{Value, json};
use sf_domain::{Issue, cloud_execution::ExecutionReport};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

struct Table {
    name: String,
    columns: Vec<String>,
    rows: Vec<Value>,
}
fn problem(text: impl Into<String>) -> Vec<Issue> {
    vec![issue("execution_report_format", text, None)]
}
fn cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    }
}
fn field(v: &Value, p: &str) -> Value {
    v.pointer(p).cloned().unwrap_or(Value::Null)
}
fn prefix(report: &Value, session: Option<&Value>) -> Value {
    let mut v = json!({"report_id":report["report_id"],"project_id":report["project_id"]});
    if let Some(s) = session {
        let src = &s["source"];
        for (k, x) in [
            ("session_id", src["identity"]["session_id"].clone()),
            (
                "execution_epoch_id",
                src["identity"]["execution_epoch_id"].clone(),
            ),
            ("execution_revision", src["execution_revision"].clone()),
            ("observation_frontier", src["observation_frontier"].clone()),
        ] {
            v[k] = x;
        }
    }
    v
}
fn with_prefix(report: &Value, s: Option<&Value>, row: Value) -> Value {
    let mut v = prefix(report, s);
    v.as_object_mut()
        .unwrap()
        .extend(row.as_object().unwrap().clone());
    v
}
fn arrays<'a>(v: &'a Value, p: &str) -> &'a [Value] {
    v.pointer(p)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn collect_issues(v: &Value, path: &str, context: &Value, out: &mut Vec<Value>) {
    match v {
        Value::Object(m)
            if m.contains_key("code") && m.contains_key("message") && m.contains_key("blocks") =>
        {
            let mut row = context.clone();
            for (k, x) in [
                ("source_path", json!(path)),
                ("code", v["code"].clone()),
                ("object_ref", v["object_ref"].clone()),
                ("field_path", v["field_path"].clone()),
                ("message", v["message"].clone()),
                ("affected_conclusions", v["blocks"].clone()),
            ] {
                row[k] = x;
            }
            out.push(row);
        }
        Value::Object(m) => {
            for (k, x) in m {
                collect_issues(
                    x,
                    &format!("{path}/{}", k.replace('~', "~0").replace('/', "~1")),
                    context,
                    out,
                );
            }
        }
        Value::Array(a) => {
            for (n, x) in a.iter().enumerate() {
                collect_issues(x, &format!("{path}/{n}"), context, out);
            }
        }
        _ => {}
    }
}
fn tables(report: &Value) -> Vec<Table> {
    let contract: Value =
        serde_json::from_str(include_str!("../execution-report.contract.design.json"))
            .expect("frozen CSV contract");
    let mut out = vec![];
    for spec in contract["tables"].as_array().unwrap() {
        let name = spec["id"].as_str().unwrap();
        let session_table = contract["session_row_prefix_applies_to"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == name);
        let pre = if session_table {
            &contract["session_row_prefix"]
        } else {
            &contract["common_report_prefix"]
        };
        let mut columns: Vec<String> = pre
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().into())
            .collect();
        for c in spec["columns"].as_array().unwrap() {
            let c = c.as_str().unwrap();
            if !columns.iter().any(|x| x == c) {
                columns.push(c.into());
            }
        }
        let mut rows = vec![];
        match name {
            "report_context" => rows.push(report.clone()),
            "comparison" => {
                let c = &report["comparison"];
                if !c.is_null() {
                    let mut base = prefix(report, None);
                    for (role, path) in [
                        ("baseline", "/sessions/0/source"),
                        ("candidate", "/sessions/1/source"),
                    ] {
                        let s = field(report, path);
                        for (k, v) in [
                            ("session_id", s["identity"]["session_id"].clone()),
                            (
                                "execution_epoch_id",
                                s["identity"]["execution_epoch_id"].clone(),
                            ),
                            ("execution_revision", s["execution_revision"].clone()),
                            ("observation_frontier", s["observation_frontier"].clone()),
                        ] {
                            base[format!("{role}_{k}")] = v;
                        }
                    }
                    for x in arrays(c, "/conditions") {
                        let mut row = base.clone();
                        comparison_columns(&mut row, c, x);
                        rows.push(row);
                    }
                }
            }
            "prediction_sources" => {
                let p = &report["prediction_comparison"]["prediction"];
                if !p.is_null() {
                    let mut row = with_prefix(report, None, p.clone());
                    row["run_id"] = p["source"]["run_id"].clone();
                    for k in ["project_revision", "scenario_id", "scenario_revision"] {
                        row[if k == "project_revision" {
                            "input_project_revision"
                        } else {
                            k
                        }] = p["source"]["input_ref"][k].clone();
                    }
                    row["time_reference_json"] = p["time_reference"].clone();
                    row["window_start_ms"] = p["window"]["start_ms"].clone();
                    row["window_end_ms"] = p["window"]["end_ms"].clone();
                    row["target_quantity_json"] = p["target_quantity"].clone();
                    row["module_versions_json"] = p["module_versions"].clone();
                    rows.push(row);
                }
            }
            "prediction_accepted_by_scope" => {
                let p = &report["prediction_comparison"]["prediction"];
                for x in arrays(p, "/accepted") {
                    let mut row = accepted_columns(x);
                    row["run_id"] = p["source"]["run_id"].clone();
                    rows.push(with_prefix(report, None, row));
                }
            }
            "prediction_waiting" => {
                let p = &report["prediction_comparison"]["prediction"];
                for x in arrays(p, "/waiting") {
                    let mut row = x.clone();
                    row["run_id"] = p["source"]["run_id"].clone();
                    row["object_kind"] = x["object_ref"]["kind"].clone();
                    row["object_id"] = x["object_ref"]["id"].clone();
                    row["source_intervals_json"] = x["source_intervals"].clone();
                    rows.push(with_prefix(report, None, row));
                }
            }
            "prediction_comparison" => {
                let p = &report["prediction_comparison"];
                if !p.is_null() {
                    for c in arrays(p, "/conditions") {
                        let mut row = prefix(report, None);
                        row["prediction_run_id"] = p["prediction"]["source"]["run_id"].clone();
                        row["execution_read_origin"] = p["execution"]["read_origin"].clone();
                        row["execution_session_id"] =
                            p["execution"]["identity"]["session_id"].clone();
                        row["execution_epoch_id"] =
                            p["execution"]["identity"]["execution_epoch_id"].clone();
                        row["execution_revision"] = p["execution"]["execution_revision"].clone();
                        row["comparison_scope_json"] = p["comparison_scope"].clone();
                        comparison_columns(&mut row, p, c);
                        rows.push(row);
                    }
                }
            }
            "issues" => {
                for key in ["limitations", "comparison", "prediction_comparison"] {
                    collect_issues(
                        &report[key],
                        &format!("/{key}"),
                        &prefix(report, None),
                        &mut rows,
                    );
                }
                for (n, s) in arrays(report, "/sessions").iter().enumerate() {
                    collect_issues(
                        s,
                        &format!("/sessions/{n}"),
                        &prefix(report, Some(s)),
                        &mut rows,
                    );
                }
            }
            _ => {
                for s in arrays(report, "/sessions") {
                    let x = &s["snapshot"];
                    let mut local = vec![];
                    match name {
                        "execution_sources" => {
                            let mut row = s["source"].clone();
                            for k in ["host_context_session_id"] {
                                row[k] = s["source"]["identity"][k].clone();
                            }
                            for (k, path) in [
                                (
                                    "input_project_revision",
                                    "/project_input_ref/project_revision",
                                ),
                                ("scenario_id", "/project_input_ref/scenario_id"),
                                ("scenario_revision", "/project_input_ref/scenario_revision"),
                                ("source_mode", "/provenance/mode"),
                                ("clock_started", "/coverage/clock_started"),
                                ("observed_until_host_ms", "/coverage/observed_until_host_ms"),
                                (
                                    "requested_cutoff_reached",
                                    "/coverage/requested_cutoff_reached",
                                ),
                                ("lifecycle", "/lifecycle/state"),
                            ] {
                                row[k] = field(x, path);
                            }
                            row["goal_at_snapshot"] = x["goal_at_snapshot"].clone();
                            row["goal_at_cutoff"] = x["goal_at_cutoff"].clone();
                            row["cloud_published_revision"] =
                                s["source"]["delivery"]["cloud_published_revision"].clone();
                            local.push(row);
                        }
                        "accepted_by_scope" => {
                            for a in arrays(s, "/metrics/accepted") {
                                local.push(accepted_columns(a));
                            }
                        }
                        "command_counts" => local.push(s["metrics"]["commands"].clone()),
                        "waiting" => local.extend_from_slice(arrays(s, "/metrics/waiting")),
                        "work_timings" => local.extend_from_slice(arrays(x, "/timings")),
                        "receipt_admissions" => {
                            for a in arrays(x, "/receipt_admissions") {
                                let mut row = a.clone();
                                row["admission_execution_revision"] =
                                    a["execution_revision"].clone();
                                row.as_object_mut().unwrap().remove("execution_revision");
                                local.push(row);
                            }
                        }
                        "logical_work_and_attempts" => {
                            for (kind, path) in
                                [("logical_work", "/logical_work"), ("attempt", "/attempts")]
                            {
                                for a in arrays(x, path) {
                                    let r = if kind == "attempt" {
                                        &a["attempt"]
                                    } else {
                                        &a["current_attempt"]
                                    };
                                    let d = if kind == "attempt" {
                                        &a["dispatch_decision"]
                                    } else {
                                        &a["latest_dispatch_decision"]
                                    };
                                    let mut row = json!({"kind":kind,"logical_job_id":if kind=="attempt"{r["logical_job_id"].clone()}else{a["logical_job_id"].clone()},"command_id":r["command_id"],"phase_id":r["phase_id"],"attempt_ordinal":r["attempt_ordinal"],"selected_binding_id":d["selected_binding_id"],"target_device_boot_id":if kind=="attempt"{a["target_device_boot_id"].clone()}else{Value::Null},"state":a["state"],"decision_host_ms":d["decision_host_ms"],"decision_reason":d["reason"],"candidate_details_json":d["candidates"]});
                                    if row["selected_binding_id"].is_null() {
                                        row["selected_binding_id"] =
                                            r["selected_binding_id"].clone();
                                    }
                                    local.push(row);
                                }
                            }
                        }
                        "resource_claims" => {
                            for p in arrays(x, "/business_facts/resource_claims") {
                                let c = &p["claim"];
                                for (n, i) in arrays(c, "/intervals").iter().enumerate() {
                                    let open = p["open_interval_index"].as_u64() == Some(n as u64);
                                    local.push(json!({"resource_id":c["resource_id"],"job_id":c["job_id"],"units":c["units"],"interval_index":n,"interval_start_ms":i["start_ms"],"interval_end_ms":i["end_ms"],"is_open":open,"view_boundary":if open{p["view_boundary"].clone()}else{json!("actual_release")}}));
                                }
                            }
                        }
                        "state_history" => {
                            for r in arrays(s, "/history/records") {
                                let o = &r["object"];
                                let d = &o["data"];
                                let key = match o["kind"].as_str().unwrap() {
                                    "job" => "job_id",
                                    "logical_work" => "logical_job_id",
                                    "attempt" => "command_id",
                                    _ => "id",
                                };
                                let id = match o["kind"].as_str().unwrap() {
                                    "device" => d["binding"]["device_id"].clone(),
                                    "attempt" => d["attempt"][key].clone(),
                                    _ => d[key].clone(),
                                };
                                local.push(json!({"record_sequence":r["record_sequence"],"record_execution_revision":r["execution_revision"],"at_host_ms":r["at_host_ms"],"object_kind":o["kind"],"object_id":id,"typed_state_json":d}));
                            }
                        }
                        "module_versions_and_rights" => {
                            for m in arrays(s, "/module_versions") {
                                local.push(json!({"row_kind":"module_version","module_id":m["id"],"module_version":m["version"]}));
                            }
                            for a in arrays(s, "/asset_rights") {
                                let mut row = a["rights"].clone();
                                row["row_kind"] = json!("asset_rights");
                                row["module_id"] = a["module_ref"]["id"].clone();
                                row["module_version"] = a["module_ref"]["version"].clone();
                                local.push(row);
                            }
                        }
                        _ => unreachable!("all frozen CSV tables mapped"),
                    }
                    rows.extend(local.into_iter().map(|r| with_prefix(report, Some(s), r)));
                }
            }
        }
        out.push(Table {
            name: name.into(),
            columns,
            rows,
        });
    }
    out
}
fn accepted_columns(a: &Value) -> Value {
    json!({"product_ref":a["scope"]["product_ref"],"unit":a["scope"]["unit"],"destination":a["scope"]["destination"],"acceptance_stage":a["scope"]["stage"],"known":!a["accepted_quantity"].is_null(),"coefficient":a["accepted_quantity"]["value"]["coefficient"],"decimal_places":a["accepted_quantity"]["value"]["decimal_places"],"missing_conditions":a["missing_conditions"]})
}
fn comparison_columns(row: &mut Value, p: &Value, c: &Value) {
    for (k, v) in [
        ("status", p["status"].clone()),
        ("condition_key", c["key"].clone()),
        ("matches", c["matches"].clone()),
        ("detail", c["detail"].clone()),
        ("completion_delta_ms", p["completion_delta_ms"].clone()),
        (
            "accepted_delta_direction",
            p["accepted_delta"]["direction"].clone(),
        ),
        (
            "accepted_delta_coefficient",
            p["accepted_delta"]["magnitude"]["value"]["coefficient"].clone(),
        ),
        (
            "accepted_delta_decimal_places",
            p["accepted_delta"]["magnitude"]["value"]["decimal_places"].clone(),
        ),
        (
            "accepted_delta_unit",
            p["accepted_delta"]["magnitude"]["unit"].clone(),
        ),
    ] {
        row[k] = v;
    }
}
fn quantity(v: &Value) -> String {
    if v.is_null() {
        return "未知".into();
    }
    let q = &v["value"];
    let n = cell(&q["coefficient"]);
    let d = q["decimal_places"].as_u64().unwrap() as usize;
    let exact = if d == 0 {
        n
    } else {
        let pad = format!("{:0>width$}", n, width = d + 1);
        format!("{}.{}", &pad[..pad.len() - d], &pad[pad.len() - d..])
    };
    format!("{exact} {}", cell(&v["unit"]))
}
fn known(v: &Value, unit: &str) -> String {
    if v.is_null() {
        "未知，参见缺失条件".into()
    } else {
        format!("{} {unit}", cell(v))
    }
}
fn reasons(v: &Value) -> String {
    let rows = arrays(v, "");
    if rows.is_empty() {
        "无".into()
    } else {
        rows.iter()
            .map(|x| cell(&x["message"]))
            .collect::<Vec<_>>()
            .join("；")
    }
}
fn reading(v: &Value) -> Vec<(String, Vec<String>)> {
    let mut out=vec![("报告与事实范围".into(),vec![format!("报告 {}；工程 {}；导出修订 {}；生成时间 {}",cell(&v["report_id"]),cell(&v["project_id"]),cell(&v["export_project_revision"]),cell(&v["generated_at_utc"])),"同源实际执行观察；软件在环不等于工厂实测。数量、命令完成与产品验收分开，未知不会补零。".into()])];
    for s in arrays(v, "/sessions") {
        let x = &s["snapshot"];
        let m = &s["metrics"];
        let mut lines = vec![
            format!(
                "{} / {}",
                cell(&s["project_title"]),
                cell(&s["scenario_title"])
            ),
            format!(
                "来源 {}；会话 {}；epoch {}；R={}，F={}；来源模式 {}",
                cell(&s["source"]["read_origin"]),
                cell(&s["source"]["identity"]["session_id"]),
                cell(&s["source"]["identity"]["execution_epoch_id"]),
                cell(&s["source"]["execution_revision"]),
                cell(&s["source"]["observation_frontier"]),
                cell(&x["provenance"]["mode"])
            ),
            format!(
                "Host观察到 {} ms；状态 {}；固定目标 {}；首次达标 {}",
                cell(&x["coverage"]["observed_until_host_ms"]),
                cell(&x["lifecycle"]["state"]),
                quantity(&m["target_quantity"]),
                known(&m["target_reached_at_host_ms"], "Host ms")
            ),
        ];
        for a in arrays(m, "/accepted") {
            lines.push(format!(
                "产品 {} → {} / {}：接受 {}。缺失条件：{}",
                cell(&a["scope"]["product_ref"]),
                cell(&a["scope"]["destination"]),
                cell(&a["scope"]["stage"]),
                quantity(&a["accepted_quantity"]),
                reasons(&a["missing_conditions"])
            ));
        }
        let c = &m["commands"];
        lines.push(format!(
            "尝试 {}；Started {}；设备Completed {}；已接受逻辑工作 {}；未决命令 {}",
            cell(&c["attempt_count"]),
            cell(&c["started_attempt_count"]),
            cell(&c["device_completed_attempt_count"]),
            cell(&c["accepted_logical_work_count"]),
            cell(&c["unresolved_command_count"])
        ));
        for w in arrays(m, "/waiting") {
            lines.push(format!(
                "逻辑工作 {}：排队 {}；开工前 {}；输出等待 {}；核对等待 {}。{}",
                cell(&w["logical_job_id"]),
                known(&w["queued_wait_ms"], "ms"),
                known(&w["pre_start_wait_ms"], "ms"),
                known(&w["output_wait_ms"], "ms"),
                known(&w["reconciliation_wait_ms"], "ms"),
                reasons(&w["missing_conditions"])
            ));
        }
        for r in arrays(m, "/resources") {
            lines.push(format!(
                "资源 {}：确认占用并集 {}；占用量 {}；物理有效工时 {}。{}",
                cell(&r["resource_id"]),
                known(&r["confirmed_occupation_ms"], "ms"),
                known(&r["claimed_unit_ms"], "unit-ms"),
                known(&r["physical_effective_work_ms"], "ms"),
                reasons(&r["missing_conditions"])
            ));
        }
        for t in arrays(x, "/timings") {
            lines.push(format!("工作 {} / 命令 {}：确认预留 {}；确认占用 {}；名义加工 {}；设备自报 {}；物理有效工时 {}。{}",cell(&t["job_id"]),cell(&t["command_id"]),known(&t["host_confirmed_reservation_ms"],"ms"),known(&t["host_confirmed_occupation_ms"],"ms"),known(&t["nominal_work_ms"],"ms"),known(&t["device_reported_duration_ms"],"ms"),known(&t["physical_effective_work_ms"],"ms"),reasons(&t["missing_conditions"])));
        }
        lines.push(format!(
            "本来源缺失条件：{}",
            reasons(&m["missing_conditions"])
        ));
        out.push((
            format!("执行来源 {}", cell(&s["source"]["identity"]["session_id"])),
            lines,
        ));
    }
    for (key, title) in [
        ("comparison", "两次执行的观察比较"),
        ("prediction_comparison", "预测与执行偏差"),
    ] {
        let c = &v[key];
        if c.is_null() {
            continue;
        }
        let mut lines = vec![format!(
            "比较状态 {}；时间差 {}；接受量变化 {}",
            cell(&c["status"]),
            known(&c["completion_delta_ms"], "ms"),
            if c["accepted_delta"].is_null() {
                "未知".into()
            } else {
                format!(
                    "{} {}",
                    cell(&c["accepted_delta"]["direction"]),
                    quantity(&c["accepted_delta"]["magnitude"])
                )
            }
        )];
        for c in arrays(c, "/conditions") {
            lines.push(format!(
                "条件 {}：{}；{}",
                cell(&c["key"]),
                if c["matches"] == true {
                    "满足"
                } else {
                    "未满足/不可直接比较"
                },
                cell(&c["detail"])
            ));
        }
        if key == "prediction_comparison" {
            let p = &c["prediction"];
            lines.push(format!("原始预测Run {}；模型虚拟时钟；窗口 {} 至 {}；已计算前沿 {}；固定目标 {}；模型首次达标 {}",cell(&p["source"]["run_id"]),cell(&p["window"]["start_ms"]),cell(&p["window"]["end_ms"]),known(&p["computed_until_sim_ms"],"模拟ms"),quantity(&p["target_quantity"]),known(&p["target_reached_at_sim_ms"],"模拟ms")));
            for a in arrays(p, "/accepted") {
                lines.push(format!(
                    "预测范围 {}：{}；{}",
                    cell(&a["scope"]),
                    quantity(&a["accepted_quantity"]),
                    reasons(&a["missing_conditions"])
                ));
            }
            for w in arrays(p, "/waiting") {
                lines.push(format!(
                    "模型对象 {}：等待并集 {}，原区间 {} 条；覆盖 {} 至 {}；{}",
                    cell(&w["object_ref"]),
                    known(&w["known_sim_ms"], "模拟ms"),
                    arrays(w, "/source_intervals").len(),
                    cell(&w["covered_from_sim_ms"]),
                    known(&w["covered_until_sim_ms"], "模拟ms"),
                    reasons(&w["missing_conditions"])
                ));
            }
        }
        lines.push(format!("比较限制：{}", reasons(&c["limitations"])));
        out.push((title.into(), lines));
    }
    out.push((
        "结论与限制".into(),
        arrays(v, "/conclusions")
            .iter()
            .map(cell)
            .chain(arrays(v, "/limitations").iter().map(cell))
            .collect(),
    ));
    out
}
struct Pdf {
    doc: PdfDocument,
    font: FontId,
    metrics: ParsedFont,
    pages: Vec<PdfPage>,
    ops: Vec<Op>,
    y: f32,
}
impl Pdf {
    fn new(bytes: &[u8]) -> Result<Self, Vec<Issue>> {
        let metrics = ParsedFont::from_bytes(bytes, 0, &mut vec![])
            .ok_or_else(|| problem("实际中文字体无法解析"))?;
        let mut doc = PdfDocument::new("超级工厂执行观察与预测比较报告");
        let font = doc.add_font(&metrics);
        Ok(Self {
            doc,
            font,
            metrics,
            pages: vec![],
            ops: vec![],
            y: 785.,
        })
    }
    fn text(&mut self, s: &str, x: f32, y: f32, size: f32) {
        self.ops.extend([
            Op::StartTextSection,
            Op::SetTextCursor {
                pos: Point { x: Pt(x), y: Pt(y) },
            },
            Op::SetFont {
                font: PdfFontHandle::External(self.font.clone()),
                size: Pt(size),
            },
            Op::ShowText {
                items: vec![TextItem::Text(s.into())],
            },
            Op::EndTextSection,
        ]);
    }
    fn page(&mut self) {
        self.text(
            &format!(
                "超级工厂执行报告 | 第 {} 页 | 未知不补零",
                self.pages.len() + 1
            ),
            48.,
            30.,
            9.,
        );
        self.pages.push(PdfPage::new(
            Mm(210.),
            Mm(297.),
            std::mem::take(&mut self.ops),
        ));
        self.y = 785.;
    }
    fn line(&mut self, s: &str, size: f32) {
        // Keep ordinary paragraphs together; long canonical appendix entries
        // may continue. Width uses the same embedded font as the writer.
        let mut needed = 1u32;
        let mut used = 0.;
        for c in s.chars() {
            let advance = self
                .metrics
                .lookup_glyph_index(c as u32)
                .and_then(|gid| self.metrics.get_or_decode_glyph(gid))
                .map_or(0., |g| g.horz_advance as f32)
                * size
                / self.metrics.pdf_font_metrics.units_per_em as f32;
            if c == '\n' || used + advance > 496. {
                needed += 1;
                used = 0.;
            }
            if c != '\n' {
                used += advance;
            }
        }
        if needed <= 8 && self.y - needed as f32 * size * 1.5 < 55. {
            self.page();
        }
        let mut line = String::new();
        let mut width = 0.;
        for c in s.chars() {
            let w = self
                .metrics
                .lookup_glyph_index(c as u32)
                .and_then(|gid| self.metrics.get_or_decode_glyph(gid))
                .map_or(0., |g| g.horz_advance as f32)
                * size
                / self.metrics.pdf_font_metrics.units_per_em as f32;
            if c == '\n' || width + w > 496. {
                if self.y < 55. {
                    self.page();
                }
                let y = self.y;
                self.text(&line, 48., y, size);
                self.y -= size * 1.5;
                line.clear();
                width = 0.;
                if c == '\n' {
                    continue;
                }
            }
            line.push(c);
            width += w;
        }
        if !line.is_empty() {
            if self.y < 55. {
                self.page();
            }
            let y = self.y;
            self.text(&line, 48., y, size);
            self.y -= size * 1.5;
        }
    }
    fn section(&mut self, s: &str) {
        if self.y < 105. {
            self.page();
        }
        self.y -= 10.;
        self.line(s, 15.);
        self.y -= 5.;
    }
    fn finish(mut self) -> Vec<u8> {
        self.page();
        self.doc
            .with_pages(self.pages)
            .save(&PdfSaveOptions::default(), &mut vec![])
    }
}
fn file(p: &str, kind: &str, bytes: Vec<u8>) -> EvaluationFile {
    EvaluationFile {
        relative_path: p.into(),
        media_type: kind.into(),
        bytes,
    }
}
pub fn render_execution_report(
    report: &ExecutionReport,
    formats: &[EvaluationOutputFormat],
    resources: &EvaluationResources,
    cancel: &AtomicBool,
) -> Result<Vec<EvaluationFile>, Vec<Issue>> {
    cancelled(cancel)?;
    if formats.is_empty() {
        return Err(problem("至少选择一种执行报告格式"));
    }
    let v = value(report);
    check_export_content(&v)?;
    let tables = tables(&v);
    let sections = reading(&v);
    let canonical = serde_json::to_vec_pretty(report).map_err(|e| problem(e.to_string()))?;
    let mut files = BTreeMap::new();
    let human = formats.iter().any(|f| {
        matches!(
            f,
            EvaluationOutputFormat::Pdf | EvaluationOutputFormat::Html
        )
    });
    if human {
        let font = resources
            .cjk_font
            .as_ref()
            .filter(|f| !f.is_empty())
            .ok_or_else(|| problem("缺少实际中文字体字节"))?;
        let license = resources
            .font_license
            .as_ref()
            .filter(|x| !x.is_empty())
            .ok_or_else(|| problem("缺少字体许可证字节"))?;
        files.insert(
            "fonts/SourceHanSansCN-Regular.otf".into(),
            file(
                "fonts/SourceHanSansCN-Regular.otf",
                "font/otf",
                font.clone(),
            ),
        );
        files.insert(
            "licenses/font.txt".into(),
            file(
                "licenses/font.txt",
                "text/plain;charset=utf-8",
                license.clone(),
            ),
        );
    }
    for f in formats {
        cancelled(cancel)?;
        match f {
            EvaluationOutputFormat::Json => {
                files.insert(
                    "report.json".into(),
                    file("report.json", "application/json", canonical.clone()),
                );
            }
            EvaluationOutputFormat::Csv => {
                for t in &tables {
                    cancelled(cancel)?;
                    let mut w = csv::WriterBuilder::new()
                        .terminator(csv::Terminator::CRLF)
                        .from_writer(vec![]);
                    w.write_record(&t.columns)
                        .map_err(|e| problem(e.to_string()))?;
                    for row in &t.rows {
                        cancelled(cancel)?;
                        w.write_record(t.columns.iter().map(|c| cell(&row[c])))
                            .map_err(|e| problem(e.to_string()))?;
                    }
                    let mut bytes = vec![0xef, 0xbb, 0xbf];
                    bytes.extend(w.into_inner().map_err(|e| problem(e.to_string()))?);
                    let name = format!("csv/{}.csv", t.name);
                    files.insert(name.clone(), file(&name, "text/csv;charset=utf-8", bytes));
                }
            }
            EvaluationOutputFormat::Html => {
                let mut html = String::from(
                    "<!doctype html><html lang=zh-CN><meta charset=utf-8><meta name=viewport content='width=device-width'><title>超级工厂执行报告</title><style>@font-face{font-family:ReportCN;src:url('fonts/SourceHanSansCN-Regular.otf')}body{font:15px/1.65 ReportCN,system-ui;color:#163347;background:#edf3f6;margin:0}main{max-width:1200px;margin:auto;padding:32px}h1{color:#075a90}section,details{padding:20px;background:white;border:1px solid #cad9e3;border-radius:6px;margin:15px 0}p,pre{white-space:pre-wrap;overflow-wrap:anywhere}summary{cursor:pointer;font-weight:bold}.scroll{overflow:auto}table{border-collapse:collapse;font-size:12px}th,td{border:1px solid #cad9e3;padding:7px;vertical-align:top;min-width:100px;max-width:400px;overflow-wrap:anywhere}th{background:#e8f1f6}</style><main><h1>超级工厂 · 执行观察与预测比较</h1>",
                );
                for (title, lines) in &sections {
                    html.push_str(&format!("<section><h2>{}</h2>", escape(title)));
                    for line in lines {
                        html.push_str(&format!("<p>{}</p>", escape(line)));
                    }
                    html.push_str("</section>");
                }
                html.push_str("<h2>同源完整明细：17 张分表</h2>");
                for t in &tables {
                    cancelled(cancel)?;
                    html.push_str(&format!("<details><summary>{} · {} 行</summary><div class=scroll><table><thead><tr>",escape(&t.name),t.rows.len()));
                    for c in &t.columns {
                        html.push_str(&format!("<th>{}</th>", escape(c)));
                    }
                    html.push_str("</tr></thead><tbody>");
                    for r in &t.rows {
                        html.push_str("<tr>");
                        for c in &t.columns {
                            html.push_str(&format!(
                                "<td>{}</td>",
                                if r[c].is_null() {
                                    "未知/不适用".into()
                                } else {
                                    escape(&cell(&r[c]))
                                }
                            ));
                        }
                        html.push_str("</tr>");
                    }
                    html.push_str("</tbody></table></div></details>");
                }
                let raw = String::from_utf8(canonical.clone())
                    .unwrap()
                    .replace('<', "\\u003c")
                    .replace('>', "\\u003e")
                    .replace('&', "\\u0026");
                html.push_str(&format!("<details><summary>完整 canonical JSON（含原历史和所有条件）</summary><pre id=raw></pre></details><script id=report-data type=application/json>{raw}</script><script>document.getElementById('raw').textContent=JSON.stringify(JSON.parse(document.getElementById('report-data').textContent),null,2)</script></main></html>"));
                files.insert(
                    "report.html".into(),
                    file("report.html", "text/html;charset=utf-8", html.into_bytes()),
                );
            }
            EvaluationOutputFormat::Pdf => {
                let mut pdf = Pdf::new(resources.cjk_font.as_ref().unwrap())?;
                let generated = printpdf::DateTime::from_unix_timestamp(
                    report.generated_at_utc.as_datetime().timestamp(),
                )
                .expect("validated report UTC timestamp");
                pdf.doc.metadata.info.creation_date = generated;
                pdf.doc.metadata.info.modification_date = generated;
                pdf.doc.metadata.info.metadata_date = generated;
                pdf.line("超级工厂执行观察与预测比较", 21.);
                for (title, lines) in &sections {
                    cancelled(cancel)?;
                    pdf.section(title);
                    for s in lines {
                        pdf.line(s, 10.5);
                    }
                }
                pdf.section("同源完整数据分表索引");
                for t in &tables {
                    pdf.line(
                        &format!(
                            "{}：{} 行；列顺序见附录 canonical 对应字段及CSV合同。",
                            t.name,
                            t.rows.len()
                        ),
                        10.,
                    );
                }
                pdf.section("完整 canonical 数据附录");
                pdf.line("以下逐条保留同一报告的来源、历史、事实、指标和比较条件；独立PDF不依赖旁边文件才能核对完整明细。字体许可随包提供。",10.);
                for line in String::from_utf8(canonical.clone()).unwrap().lines() {
                    cancelled(cancel)?;
                    pdf.line(line, 7.5);
                }
                files.insert(
                    "report.pdf".into(),
                    file("report.pdf", "application/pdf", pdf.finish()),
                );
            }
        }
    }
    cancelled(cancel)?;
    Ok(files.into_values().collect())
}
