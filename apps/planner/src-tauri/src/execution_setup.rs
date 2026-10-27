//! Native-only connection file and lifecycle. No Renderer or Agent configuration.
use crate::device_process::{DeviceProfile, DeviceToken, NativeCloudOrigin, NativeDeviceRuntime};
use crate::execution_runtime::{
    self, NativeExecutionActor, NativeExecutionConfig, NativeExecutionDevice,
};
use serde::Deserialize;
use sf_commands::execution_cloud::ExecutionCloudConnection;
use sf_commands::{
    ApiError, CloudReadConnection, ErrorCode, ExecutionPreparationRuntime, ProjectHost,
};
use sf_domain::cloud_execution::CandidateBinding;
use sf_domain::{CloudProjectId, ObjectId, ProjectId};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

fn error(code: ErrorCode, message: &str) -> ApiError {
    ApiError {
        code,
        message: message.into(),
        object_refs: vec![],
        field_errors: vec![],
        expected_revision: None,
        actual_revision: None,
    }
}

// Only deserialized from app_data_dir/v2-cloud/connection.json, never an IPC body.
// Secret fields deliberately have neither Debug nor Serialize.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateConnection {
    connection_ref: ObjectId,
    project_id: ProjectId,
    cloud_project_id: CloudProjectId,
    endpoint: String,
    reader_token: String,
    gateway_token: String,
    candidate_bindings: Vec<CandidateBinding>,
    devices: Vec<PrivateDevice>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateDevice {
    device_id: ObjectId,
    profile: PrivateProfile,
    token: String,
}
#[derive(Deserialize, PartialEq, Eq, PartialOrd, Ord)]
enum PrivateProfile {
    #[serde(rename = "processor-a")]
    ProcessorA,
    #[serde(rename = "processor-b")]
    ProcessorB,
}
impl PrivateConnection {
    fn validate_bindings(&self) -> Result<(), ApiError> {
        let candidates: BTreeSet<_> = self
            .candidate_bindings
            .iter()
            .map(|b| &b.device_id)
            .collect();
        let devices: BTreeSet<_> = self.devices.iter().map(|b| &b.device_id).collect();
        let profiles: BTreeSet<_> = self.devices.iter().map(|b| &b.profile).collect();
        if self.candidate_bindings.len() != 2
            || self.devices.len() != 2
            || candidates.len() != 2
            || candidates != devices
            || profiles.len() != 2
        {
            return Err(error(
                ErrorCode::InvalidInput,
                "Native connection needs exactly two distinct candidate devices and processor profiles",
            ));
        }
        for device in &self.devices {
            let candidate = self
                .candidate_bindings
                .iter()
                .find(|b| b.device_id == device.device_id)
                .expect("same exact device set");
            let nominal = match device.profile {
                PrivateProfile::ProcessorA => 10000,
                PrivateProfile::ProcessorB => 15000,
            };
            if candidate.nominal_work_ms.value() != nominal {
                return Err(error(
                    ErrorCode::InvalidInput,
                    "Native processor profile differs from its exact candidate duration",
                ));
            }
        }
        Ok(())
    }
}

pub(super) struct NativeExecutionInstallation {
    project_id: ProjectId,
    reader: CloudReadConnection,
    runtime: ExecutionPreparationRuntime,
    actor: NativeExecutionActor,
}
impl NativeExecutionInstallation {
    pub(super) fn load(
        host: Arc<Mutex<ProjectHost>>,
        resource_root: &Path,
        app_data_dir: &Path,
    ) -> Result<Option<Self>, ApiError> {
        let private = app_data_dir.join("v2-cloud");
        let bytes = match std::fs::read(private.join("connection.json")) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => {
                return Err(error(
                    ErrorCode::IoFailure,
                    "Cannot read the Native private cloud connection",
                ));
            }
        };
        let profile: PrivateConnection = serde_json::from_slice(&bytes).map_err(|_| {
            error(
                ErrorCode::InvalidInput,
                "Private cloud connection does not match the Native configuration contract",
            )
        })?;
        profile.validate_bindings()?;
        let reader = CloudReadConnection::https(
            profile.connection_ref.clone(),
            profile.project_id.clone(),
            profile.cloud_project_id.clone(),
            profile.endpoint.clone(),
            profile.reader_token,
            profile.candidate_bindings,
        )?;
        let cloud = Arc::new(ExecutionCloudConnection::https(
            profile.connection_ref.clone(),
            profile.project_id.clone(),
            profile.cloud_project_id,
            profile.endpoint.clone(),
            profile.gateway_token,
        )?);
        let origin =
            NativeCloudOrigin::from_native_configuration(profile.endpoint).map_err(|_| {
                error(
                    ErrorCode::InvalidInput,
                    "Native device connection requires a fixed HTTPS origin",
                )
            })?;
        let devices = Arc::new(
            NativeDeviceRuntime::from_native_paths(resource_root, &private, origin).map_err(
                |_| {
                    error(
                        ErrorCode::InvalidInput,
                        "Native device resources or private runtime directory are invalid",
                    )
                },
            )?,
        );
        let mut bindings = BTreeMap::new();
        for device in profile.devices {
            bindings.insert(
                device.device_id,
                NativeExecutionDevice {
                    profile: match device.profile {
                        PrivateProfile::ProcessorA => DeviceProfile::ProcessorA,
                        PrivateProfile::ProcessorB => DeviceProfile::ProcessorB,
                    },
                    token: DeviceToken::from_native_secret(device.token).map_err(|_| {
                        error(
                            ErrorCode::InvalidInput,
                            "Native device credential is invalid",
                        )
                    })?,
                },
            );
        }
        let (runtime, actor) = execution_runtime::start_execution_runtime(
            host,
            NativeExecutionConfig {
                cloud,
                devices,
                connection_ref: profile.connection_ref,
                bindings,
                archive_root: private.join("executions"),
            },
        )?;
        Ok(Some(Self {
            project_id: profile.project_id,
            reader,
            runtime,
            actor,
        }))
    }

    // Reader/actor construction and all I/O occurred in setup, outside the Host lock.
    // Reinstall the same queue after an authorized project departure resets the registry.
    pub(super) fn install_for_active_project(
        &self,
        host: &mut ProjectHost,
        ui: &ObjectId,
    ) -> Result<(), ApiError> {
        if host.desktop_close_state().project_id.as_ref() == Some(&self.project_id)
            && !host.execution_preparation_available()
        {
            if self.actor.is_finished() {
                return Err(error(
                    ErrorCode::ComputationFailed,
                    "Native execution worker has exited",
                ));
            }
            host.install_cloud_read_connection(ui, self.reader.clone())?;
            host.install_execution_preparation_runtime(
                ui,
                self.project_id.clone(),
                self.runtime.clone(),
            )?;
        }
        Ok(())
    }

    pub(super) fn require_settled_for_close(&self) -> Result<(), ApiError> {
        if self.actor.status().has_obligations {
            return Err(error(
                ErrorCode::InvalidInput,
                "Native execution still owns preparation, production or cleanup obligations",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use sf_commands::FilePurpose;
    use sf_domain::TaskHandle;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    fn id(s: &str) -> ObjectId {
        s.to_owned().try_into().unwrap()
    }
    fn root() -> std::path::PathBuf {
        // Both the product crate and its independent harness locate by source file.
        Path::new(file!())
            .canonicalize()
            .unwrap()
            .parent()
            .unwrap()
            .join("../../../..")
            .canonicalize()
            .unwrap()
    }
    fn directory() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "sf-native-connection-proof-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("v2-cloud")).unwrap();
        path
    }
    fn profile() -> Value {
        let candidates: Value = serde_json::from_slice(
            &std::fs::read(root().join("resources/v2-demo/candidate-bindings.json")).unwrap(),
        )
        .unwrap();
        json!({"connection_ref":"native-config-proof", "project_id":"Task8-two-device-six-units", "cloud_project_id":"native-cloud-proof", "endpoint":"https://example.invalid/", "reader_token":"finite-reader", "gateway_token":"finite-gateway", "candidate_bindings":candidates,
            "devices":[{"device_id":"L1-D01","profile":"processor-a","token":"finite-device-a"},{"device_id":"L1-D02","profile":"processor-b","token":"finite-device-b"}]})
    }
    fn host() -> (Arc<Mutex<ProjectHost>>, ObjectId) {
        let ui = id("native-config-ui");
        (
            Arc::new(Mutex::new(ProjectHost::new(
                id("native-config-host"),
                ui.clone(),
            ))),
            ui,
        )
    }
    fn load(
        value: &Value,
        host: Arc<Mutex<ProjectHost>>,
    ) -> Result<Option<NativeExecutionInstallation>, ApiError> {
        let path = directory();
        std::fs::write(
            path.join("v2-cloud/connection.json"),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
        NativeExecutionInstallation::load(host, &root().join("resources"), &path)
    }
    fn open_demo(host: &mut ProjectHost, ui: &ObjectId) {
        let selected = host
            .select_directory(
                ui,
                FilePurpose::ImportPackage,
                root().join("resources/v2-demo/project"),
            )
            .unwrap();
        let call = |host: &mut ProjectHost, operation: &str, params: Value| {
            host.dispatch(ui, serde_json::from_value(json!({"api_version":"0.1","request_id":format!("config-{}",NEXT.fetch_add(1,Ordering::Relaxed)),"project_id":null,"scope":{"kind":"workspace"},"operation":operation,"params":params})).unwrap())
        };
        let imported = call(
            host,
            "import_package",
            json!({"selected_file_ref":selected,"import_policy":"open_project","base_revision":null}),
        );
        assert_eq!(imported["outcome"]["status"], "ok", "{imported}");
        let task: TaskHandle = serde_json::from_value(imported["outcome"]["data"].clone()).unwrap();
        let mut work = host.drain_io_work();
        host.finish_io(work.pop().unwrap().run());
        let adopted = call(
            host,
            "apply_commands",
            json!({"base_revision":null,"commands":[{"kind":"adopt_imported_draft","data":{"task_id":task.id,"choices":{"chosen_policy":"open_project","objects_to_create":[],"module_versions":[],"reused_bindings":{"item_types":{},"resources":{},"targets":{},"ports":{}},"selected_assets":[],"accept_missing_as_draft":false,"departure":"discard_after_tasks_terminal"}}}]}),
        );
        assert_eq!(adopted["outcome"]["status"], "ok", "{adopted}");
    }
    #[test]
    fn missing_private_file_leaves_the_original_app_unconfigured() {
        let (host, _) = host();
        assert!(
            NativeExecutionInstallation::load(host, Path::new("missing-resources"), &directory())
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn unknown_private_fields_are_rejected_without_echoing_credentials() {
        let (host, _) = host();
        let mut p = profile();
        p["untrusted_secret_field"] = json!("do-not-echo-value");
        let e = match load(&p, host) {
            Err(e) => e,
            Ok(_) => panic!("unknown configuration field accepted"),
        };
        assert!(matches!(e.code, ErrorCode::InvalidInput));
        assert!(!e.message.contains("do-not-echo"));
        assert!(!e.message.contains("untrusted_secret_field"));
    }
    #[test]
    fn exact_candidate_set_profiles_and_nominal_durations_are_required() {
        for change in 0..3 {
            let mut p = profile();
            match change {
                0 => p["devices"][1]["device_id"] = json!("L1-D01"),
                1 => p["devices"][1]["profile"] = json!("processor-a"),
                _ => p["candidate_bindings"][1]["nominal_work_ms"] = json!(10000),
            }
            let parsed: PrivateConnection = serde_json::from_value(p).unwrap();
            assert!(matches!(
                parsed.validate_bindings().unwrap_err().code,
                ErrorCode::InvalidInput
            ));
        }
    }
    #[test]
    fn private_plaintext_origin_is_not_admitted_to_native_runtime() {
        let (host, _) = host();
        let mut p = profile();
        p["endpoint"] = json!("http://127.0.0.1:9/");
        assert!(matches!(
            load(&p, host),
            Err(ApiError {
                code: ErrorCode::InvalidInput,
                ..
            })
        ));
    }
    #[test]
    fn real_saved_project_installs_and_reinstalls_the_same_native_queue_without_io() {
        let (host, ui) = host();
        let installation = load(&profile(), host.clone()).unwrap().unwrap();
        let mut guard = host.lock().unwrap();
        installation
            .install_for_active_project(&mut guard, &ui)
            .unwrap();
        assert!(!guard.execution_preparation_available());
        open_demo(&mut guard, &ui);
        installation
            .install_for_active_project(&mut guard, &ui)
            .unwrap();
        assert!(guard.execution_preparation_available());
        installation.require_settled_for_close().unwrap();
        open_demo(&mut guard, &ui);
        assert!(!guard.execution_preparation_available());
        installation
            .install_for_active_project(&mut guard, &ui)
            .unwrap();
        assert!(guard.execution_preparation_available());
        assert!(!installation.actor.is_finished());
        assert!(installation.actor.status().children.is_empty());
        assert!(!installation.actor.status().has_obligations);
    }
}
