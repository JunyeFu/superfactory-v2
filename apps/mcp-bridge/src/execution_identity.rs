//! Consumer correlation over actual typed replies; no execution or retry engine.
use serde_json::Value;
use std::{collections::BTreeMap, io};

fn same(left: &Value, right: &Value, reason: &str) -> io::Result<()> {
    if left != right {
        Err(io::Error::new(io::ErrorKind::InvalidData, reason))
    } else {
        Ok(())
    }
}

pub fn validate_view(selection: &Value, result: &Value) -> io::Result<()> {
    same(
        &selection["kind"],
        &result["kind"],
        "Execution selection/result kind differs",
    )?;
    let data = &result["data"];
    match selection["kind"].as_str() {
        Some("inputs") => same(
            &selection["identity"],
            &data["identity"],
            "Frozen execution identity differs",
        ),
        Some("observations" | "history") => {
            let query = &selection["query"];
            for key in ["identity", "execution_revision"] {
                same(&query[key], &data[key], "Execution page identity/R differs")?;
            }
            let keys: &[&str] = if selection["kind"] == "observations" {
                &["at_host_ms"]
            } else {
                &["host_from_ms", "host_until_ms"]
            };
            for key in keys {
                same(&query[key], &data[key], "Execution page time differs")?;
            }
            Ok(())
        }
        Some("request") => same(
            &selection["originating_request_id"],
            &data["originating_request_id"],
            "Recovered request differs",
        ),
        _ => Ok(()),
    }
}

pub fn validate_reply(request: &Value, response: &Value) -> io::Result<()> {
    if response["outcome"]["status"] != "ok" {
        return Ok(());
    }
    let data = &response["outcome"]["data"];
    let params = &request["params"];
    match request["operation"].as_str() {
        Some(
            "start_execution_session"
            | "read_cloud_production"
            | "export_execution_report"
            | "export_cloud_data",
        ) => {
            same(
                &data["scope"],
                &serde_json::json!({"kind":"edit","data":{"kind":"project","data":request["project_id"]}}),
                "Task admission Project scope differs",
            )?;
            if let Some(revision) = params
                .get("base_revision")
                .or_else(|| params.get("revision"))
            {
                same(
                    &data["input_revision"],
                    revision,
                    "Task admission frozen revision differs",
                )?;
            }
            Ok(())
        }
        Some("get_task") => same(
            &params["task_id"],
            &data["task"]["id"],
            "TaskId differs from requested Task",
        ),
        Some("get_execution_snapshot") => {
            same(
                &params["identity"],
                &data["snapshot"]["identity"],
                "Snapshot execution identity differs",
            )?;
            if params["selection"]["kind"] == "revision" {
                same(
                    &params["selection"]["execution_revision"],
                    &data["snapshot"]["execution_revision"],
                    "Snapshot is not selected exact R",
                )?;
            }
            Ok(())
        }
        Some("control_execution_session") => {
            same(
                &params["identity"],
                &data["identity"],
                "Control execution identity differs",
            )?;
            same(
                &params["action"],
                &data["action"],
                "Control acknowledged another action",
            )
        }
        Some("read_execution_view") => {
            validate_view(&params["selection"], data)?;
            if data["kind"] == "request" {
                same(
                    &data["data"]["host_context_session_id"],
                    &response["context"]["host_instance_id"],
                    "Recovery belongs to another actual Host",
                )?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Task binding is learned only from admission or an actual H09 response.
#[derive(Default)]
pub struct ExecutionObservations(BTreeMap<String, (Value, Value, Value, Value)>);
impl ExecutionObservations {
    fn bind(
        &mut self,
        task: &Value,
        operation: &Value,
        request: &Value,
        source: Option<&Value>,
    ) -> io::Result<()> {
        let id = task["id"]
            .as_str()
            .ok_or_else(|| io::Error::other("Task admission has no ID"))?;
        let mut source = source.cloned().unwrap_or(Value::Null);
        if let Some((op, original, old, original_source)) = self.0.get(id) {
            same(operation, op, "Task originating operation changed")?;
            same(request, original, "Task originating request changed")?;
            for key in ["scope", "kind", "input_revision"] {
                same(&task[key], &old[key], "Frozen Task input changed")?;
            }
            if source.is_null() {
                source = original_source.clone();
            }
        }
        self.0.insert(
            id.into(),
            (operation.clone(), request.clone(), task.clone(), source),
        );
        Ok(())
    }
    pub fn observe(&mut self, request: &Value, response: &Value) -> io::Result<()> {
        if response["outcome"]["status"] != "ok" {
            return Ok(());
        }
        let data = &response["outcome"]["data"];
        match request["operation"].as_str() {
            Some(
                "start_execution_session"
                | "read_cloud_production"
                | "export_execution_report"
                | "export_cloud_data",
            ) => self.bind(
                data,
                &request["operation"],
                &request["request_id"],
                Some(request),
            ),
            Some("read_execution_view")
                if data["kind"] == "request" && data["data"]["state"] == "registered" =>
            {
                let recovered = &data["data"];
                self.bind(
                    &recovered["task"],
                    &recovered["operation"],
                    &recovered["originating_request_id"],
                    None,
                )
            }
            Some("get_task") => {
                let task = &data["task"];
                if self.0.contains_key(task["id"].as_str().unwrap_or_default()) {
                    self.bind(
                        task,
                        &data["originating_operation"],
                        &data["originating_request_id"],
                        None,
                    )?;
                    let source = &self.0[task["id"].as_str().unwrap()].3;
                    if !source.is_null() && !data["result"].is_null() {
                        let result = &data["result"]["data"];
                        let params = &source["params"];
                        match source["operation"].as_str() {
                            Some("read_cloud_production") => {
                                let selection = &params["selection"];
                                if selection["kind"] == "data_query" {
                                    same(
                                        &result["kind"],
                                        &Value::from("query_result"),
                                        "Cloud query output kind differs",
                                    )?;
                                    let query = &selection["query"];
                                    let page = &result["data"];
                                    same(
                                        &page["dataset"],
                                        &query["dataset"],
                                        "Cloud dataset differs",
                                    )?;
                                    if query["dataset"] == "execution" {
                                        same(
                                            &page["selected_epoch"],
                                            &query["identity"]["execution_epoch_id"],
                                            "Cloud query epoch differs",
                                        )?;
                                        if query["selection"]["kind"] == "revision" {
                                            same(
                                                &page["selected_execution_revision"],
                                                &query["selection"]["execution_revision"],
                                                "Cloud query exact R differs",
                                            )?;
                                        }
                                    } else {
                                        same(
                                            &page["dataset_id"],
                                            &query["dataset_id"],
                                            "Telemetry dataset differs",
                                        )?;
                                    }
                                } else if selection["kind"] == "configuration" {
                                    same(
                                        &result["kind"],
                                        &selection["kind"],
                                        "Configuration output kind differs",
                                    )?;
                                    for key in ["scenario_id", "scenario_revision"] {
                                        same(
                                            &result["data"][key],
                                            &selection[key],
                                            "Configuration Scenario revision differs",
                                        )?;
                                    }
                                    same(
                                        &result["data"]["project_revision"],
                                        &selection["base_revision"],
                                        "Configuration Project revision differs",
                                    )?;
                                } else if selection["kind"] != "snapshot" {
                                    validate_view(selection, result)?;
                                } else {
                                    same(
                                        &result["kind"],
                                        &selection["kind"],
                                        "Cloud snapshot kind differs",
                                    )?;
                                    same(
                                        &result["data"]["snapshot"]["identity"],
                                        &selection["identity"],
                                        "Cloud snapshot identity differs",
                                    )?;
                                    if selection["selection"]["kind"] == "revision" {
                                        same(
                                            &result["data"]["snapshot"]["execution_revision"],
                                            &selection["selection"]["execution_revision"],
                                            "Cloud snapshot exact R differs",
                                        )?;
                                    }
                                }
                            }
                            Some("export_cloud_data") => {
                                for key in ["query", "selection"] {
                                    same(
                                        &result[key],
                                        &params[key],
                                        "Whole export frozen query/selection differs",
                                    )?;
                                }
                                same(
                                    &result["destination"],
                                    &params["selected_destination_ref"],
                                    "Export destination differs",
                                )?;
                            }
                            Some("export_execution_report") => {
                                same(
                                    &result["destination"],
                                    &params["selected_destination_ref"],
                                    "Report destination differs",
                                )?;
                                same(
                                    &result["prediction"],
                                    &params["prediction"],
                                    "Report prediction source differs",
                                )?;
                                let sources = params["sources"].as_array().unwrap();
                                let actual = result["sources"].as_array().unwrap();
                                if sources.len() != actual.len() {
                                    return Err(io::Error::other("Report source count differs"));
                                }
                                for (original, actual) in sources.iter().zip(actual) {
                                    for key in ["read_origin", "identity", "execution_revision"] {
                                        same(
                                            &original[key],
                                            &actual[key],
                                            "Report exact source differs",
                                        )?;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
