use super::host_definitions::validate_definition;
use super::*;
use std::path::Path;

/// Immutable bytes loaded by trusted native startup. They are not Project state.
#[derive(Clone)]
pub(super) struct BundledModule {
    original: ModulePackage,
    pub package: ModulePackage,
    pub additions: Vec<ArchiveAddition>,
}

#[derive(Default)]
pub(super) struct LibraryImports {
    pub modules: BTreeMap<(String, String), ModulePackage>,
    pub additions: BTreeMap<String, ArchiveAddition>,
}

fn library_error(message: impl Into<String>) -> ApiError {
    error(
        ErrorCode::InvalidInput,
        &message.into(),
        Some("/module_ref"),
    )
}

fn package_prefix(reference: &ModuleRef) -> String {
    let hex = |value: &str| {
        value
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    format!(
        "bundled/{}/{}/",
        hex(reference.id.as_str()),
        hex(&String::from(reference.version.clone()))
    )
}

pub(super) fn relocate_template_subset(
    source: &ModulePackage,
    subset: &TemplateSubset,
) -> TemplateSubset {
    let reference = ModuleRef {
        id: source.definition.id.clone(),
        version: source.definition.version.clone(),
    };
    let prefix = package_prefix(&reference);
    let ModulePayload::Templates(template) = &source.definition.payload else {
        return subset.clone();
    };
    if !template.project.as_str().starts_with(&prefix) {
        return subset.clone();
    }
    let mut subset = subset.clone();
    if let Some(adoption) = &mut subset.adoption {
        let path = format!("{prefix}{}", adoption.source_archive.path.as_str());
        if source.files.iter().any(|file| file.path.as_str() == path) {
            adoption.source_archive.path = RelativeFilePath::try_from(path).unwrap();
        }
    }
    subset
}

fn read_library_file(root: &Path, relative: &RelativeFilePath) -> Result<Vec<u8>, ApiError> {
    let path = root
        .join(relative.as_str())
        .canonicalize()
        .map_err(|failure| {
            error(
                ErrorCode::MissingReference,
                &format!("Bundled file {}: {failure}", relative.as_str()),
                None,
            )
        })?;
    if !path.starts_with(root) {
        return Err(library_error(
            "Bundled file escapes the trusted candidate directory",
        ));
    }
    std::fs::read(path).map_err(|failure| error(ErrorCode::IoFailure, &failure.to_string(), None))
}

fn read_package(root: &Path) -> Result<ModulePackage, ApiError> {
    let bytes = read_library_file(
        root,
        &RelativeFilePath::try_from("module.json".to_owned()).unwrap(),
    )?;
    let document: Versioned<ModulePackage> = serde_json::from_slice(&bytes)
        .map_err(|failure| library_error(format!("Bundled ModulePackage: {failure}")))?;
    validate_definition(&document.data.definition)?;
    Ok(document.data)
}

fn relocate_package(original: &ModulePackage, root: &Path) -> Result<BundledModule, ApiError> {
    let reference = ModuleRef {
        id: original.definition.id.clone(),
        version: original.definition.version.clone(),
    };
    let prefix = package_prefix(&reference);
    let relocated_path = |path: &RelativeFilePath| {
        RelativeFilePath::try_from(format!("{prefix}{}", path.as_str())).unwrap()
    };
    let asset_ids: BTreeMap<_, _> = original
        .assets
        .iter()
        .map(|asset| {
            (
                asset.id.clone(),
                AssetId::try_from(format!("{prefix}{}", asset.id.as_str())).unwrap(),
            )
        })
        .collect();
    let mut package = original.clone();
    for file in &mut package.files {
        file.path = relocated_path(&file.path);
    }
    for asset in &mut package.assets {
        asset.path = relocated_path(&asset.path);
        asset.id = asset_ids[&asset.id].clone();
    }
    fn relocate_payload_assets(value: &mut Value, original: &[AssetRef], relocated: &[AssetRef]) {
        if let Ok(asset) = serde_json::from_value::<AssetRef>(value.clone()) {
            if let Some(index) = original
                .iter()
                .position(|declared| exact_data(declared, &asset))
            {
                *value = serde_json::to_value(&relocated[index]).unwrap();
            }
            return;
        }
        match value {
            Value::Object(fields) => {
                for child in fields.values_mut() {
                    relocate_payload_assets(child, original, relocated);
                }
            }
            Value::Array(values) => {
                for child in values {
                    relocate_payload_assets(child, original, relocated);
                }
            }
            _ => {}
        }
    }
    let mut payload = serde_json::to_value(&package.definition.payload).unwrap();
    relocate_payload_assets(&mut payload, &original.assets, &package.assets);
    package.definition.payload = serde_json::from_value(payload)
        .expect("AssetRef-only relocation preserves the typed payload");
    for right in package
        .rights
        .iter_mut()
        .chain(&mut package.definition.contribution.assets)
    {
        if let Some(id) = asset_ids.get(&right.asset_ref) {
            right.asset_ref = id.clone();
        }
    }
    if let ModulePayload::Templates(template) = &mut package.definition.payload {
        template.project = relocated_path(&template.project);
        if let Some(path) = &mut template.generation_profile {
            *path = relocated_path(path);
        }
    }
    let target = ObjectRef {
        kind: ObjectKind::Module,
        id: ObjectId::try_from(reference.id.as_str().to_owned()).unwrap(),
    };
    let mut additions = BTreeMap::new();
    for file in &original.files {
        let original_bytes = read_library_file(root, &file.path)?;
        let mut source_project = if matches!(file.kind, DocumentKind::Asset) {
            serde_json::from_slice::<Versioned<ProjectState>>(&original_bytes).ok()
        } else {
            None
        };
        let bytes = if matches!(file.kind, DocumentKind::Template) || source_project.is_some() {
            let relocated = if let Some(project) = &mut source_project {
                for adoption in &mut project.data.template_adoptions {
                    let path = relocated_path(&adoption.source_archive.path);
                    if package.files.iter().any(|file| file.path == path) {
                        adoption.source_archive.path = path;
                    }
                }
                encode_document(&project.data)
            } else {
                let subset: Versioned<TemplateSubset> = serde_json::from_slice(&original_bytes)
                    .map_err(|failure| {
                        library_error(format!("Bundled TemplateSubset: {failure}"))
                    })?;
                encode_document(&relocate_template_subset(&package, &subset.data))
            };
            let raw_path = RelativeFilePath::try_from(format!(
                "{prefix}original-files/{}",
                file.path.as_str()
            ))
            .unwrap();
            additions.insert(
                raw_path.as_str().to_owned(),
                ArchiveAddition {
                    entry: DocumentEntry {
                        role: DocumentRole::SupportingFile,
                        module_ref: None,
                        object_ref: target.clone(),
                        revision: None,
                        file_ref: FileRef {
                            path: raw_path,
                            kind: DocumentKind::Asset,
                        },
                        required: true,
                    },
                    bytes: original_bytes,
                },
            );
            relocated
        } else {
            original_bytes
        };
        let path = relocated_path(&file.path);
        let asset = original.assets.iter().find(|asset| asset.path == file.path);
        let entry = DocumentEntry {
            role: if asset.is_some() {
                DocumentRole::Asset
            } else {
                DocumentRole::SupportingFile
            },
            module_ref: None,
            object_ref: asset
                .map(|asset| ObjectRef {
                    kind: ObjectKind::Asset,
                    id: ObjectId::try_from(asset_ids[&asset.id].as_str().to_owned()).unwrap(),
                })
                .unwrap_or_else(|| target.clone()),
            revision: None,
            file_ref: FileRef {
                path: path.clone(),
                kind: file.kind.clone(),
            },
            required: true,
        };
        if additions
            .insert(path.as_str().to_owned(), ArchiveAddition { entry, bytes })
            .is_some()
        {
            return Err(library_error(
                "Bundled package declares duplicate file paths",
            ));
        }
    }
    for asset in &original.assets {
        if !original
            .files
            .iter()
            .any(|file| file.path == asset.path && matches!(file.kind, DocumentKind::Asset))
        {
            return Err(library_error(
                "Bundled asset requires its actual declared Asset file",
            ));
        }
        if !original
            .rights
            .iter()
            .any(|right| right.asset_ref == asset.id)
            || !original.definition.contribution.assets.iter().any(|right| {
                original
                    .rights
                    .iter()
                    .any(|actual| actual.asset_ref == asset.id && exact_data(right, actual))
            })
        {
            return Err(library_error(
                "Bundled asset rights must match the actual contribution declaration",
            ));
        }
    }
    let path = RelativeFilePath::try_from(format!("{prefix}module.json")).unwrap();
    let original_path =
        RelativeFilePath::try_from(format!("{prefix}original-module.json")).unwrap();
    additions.insert(
        original_path.as_str().to_owned(),
        ArchiveAddition {
            entry: DocumentEntry {
                role: DocumentRole::SupportingFile,
                module_ref: None,
                object_ref: target.clone(),
                revision: None,
                file_ref: FileRef {
                    path: original_path,
                    kind: DocumentKind::Asset,
                },
                required: true,
            },
            bytes: read_library_file(
                root,
                &RelativeFilePath::try_from("module.json".to_owned()).unwrap(),
            )?,
        },
    );
    additions.insert(
        path.as_str().to_owned(),
        ArchiveAddition {
            entry: DocumentEntry {
                role: DocumentRole::ModuleDefinition,
                module_ref: Some(reference),
                object_ref: target,
                revision: None,
                file_ref: FileRef {
                    path,
                    kind: DocumentKind::Module,
                },
                required: true,
            },
            bytes: encode_document(&package),
        },
    );
    Ok(BundledModule {
        original: original.clone(),
        package,
        additions: additions.into_values().collect(),
    })
}

impl ProjectHost {
    /// Read only an exact current-Project selection, an actual package declared
    /// by its loaded archive/additions, or a trusted registered library source.
    pub(super) fn readable_project_module(
        &self,
        reference: &ModuleRef,
    ) -> Result<ModulePackage, ApiError> {
        let key = module_key(reference);
        let selected = self.project.as_ref().is_some_and(|project| {
            project
                .module_refs
                .iter()
                .any(|member| module_key(member) == key)
        });
        let declared = |entry: &DocumentEntry| {
            matches!(entry.file_ref.kind, DocumentKind::Module)
                && matches!(entry.object_ref.kind, ObjectKind::Module)
                && entry.object_ref.id.as_str() == reference.id.as_str()
                && entry
                    .module_ref
                    .as_ref()
                    .is_some_and(|member| module_key(member) == key)
        };
        let archived = self.additions.iter().any(|file| declared(&file.entry))
            || self.backing.as_ref().is_some_and(|archive| {
                archive.manifest.documents.iter().any(|entry| {
                    declared(entry) && archive.files.contains_key(entry.file_ref.path.as_str())
                })
            });
        if selected || archived {
            return self.modules.get(&key).cloned().ok_or_else(|| {
                error(
                    ErrorCode::MissingReference,
                    "The declared exact module package is not loaded",
                    Some("/params/source/data/module_ref"),
                )
            });
        }
        let bundled=self.bundled_modules.get(&key).ok_or_else(|| {
            error(ErrorCode::MissingReference,"Exact module is not selected, declared by the current archive, or registered in the trusted library",Some("/params/source/data/module_ref"))
        })?;
        if self
            .modules
            .get(&key)
            .is_some_and(|current| !exact_data(current, &bundled.package))
        {
            return Err(error(
                ErrorCode::InvalidInput,
                "Current exact module conflicts with the registered library contents",
                Some("/params/source/data/module_ref"),
            ));
        }
        Ok(bundled.package.clone())
    }
    /// Trusted native startup registration; never accepts a wire path or edits a Project.
    /// The directory contains canonical module.json and its real declared files.
    pub fn register_bundled_template_directory(
        &mut self,
        directory: &Path,
    ) -> Result<(), ApiError> {
        self.register_bundled_directory(directory, true)
    }

    /// Registers one canonical category package and its declared exact dependencies.
    /// Startup registration is read-only with respect to the active Project.
    pub fn register_bundled_module_directory(&mut self, directory: &Path) -> Result<(), ApiError> {
        self.register_bundled_directory(directory, false)
    }

    fn register_bundled_directory(
        &mut self,
        directory: &Path,
        template_only: bool,
    ) -> Result<(), ApiError> {
        let root = directory
            .canonicalize()
            .map_err(|failure| error(ErrorCode::IoFailure, &failure.to_string(), None))?;
        let template = read_package(&root)?;
        if template_only && !matches!(template.definition.payload, ModulePayload::Templates(_)) {
            return Err(library_error(
                "Bundled candidate root must be a Templates ModulePackage",
            ));
        }
        let mut packages = BTreeMap::new();
        for file in &template.files {
            // Actual canonical dependencies are declared as supporting files in
            // the template, with each ModulePackage's own directory as its root.
            if !file.path.as_str().ends_with("/module.json") {
                continue;
            }
            let bytes = read_library_file(&root, &file.path)?;
            let document: Versioned<ModulePackage> = serde_json::from_slice(&bytes)
                .map_err(|failure| library_error(format!("Bundled dependency: {failure}")))?;
            validate_definition(&document.data.definition)?;
            let module_root = root
                .join(file.path.as_str())
                .parent()
                .unwrap()
                .canonicalize()
                .map_err(|failure| error(ErrorCode::IoFailure, &failure.to_string(), None))?;
            let package = relocate_package(&document.data, &module_root)?;
            let key = (
                package.package.definition.id.as_str().to_owned(),
                String::from(package.package.definition.version.clone()),
            );
            if packages.insert(key, package).is_some() {
                return Err(library_error("Duplicate bundled dependency ID/version"));
            }
        }
        let declared_definitions =
            RelativeFilePath::try_from("definitions.json".to_owned()).unwrap();
        if template
            .files
            .iter()
            .any(|file| file.path == declared_definitions)
        {
            let definitions: Vec<ModuleDefinition> =
                serde_json::from_slice(&read_library_file(&root, &declared_definitions)?).map_err(
                    |failure| library_error(format!("Bundled exact definitions: {failure}")),
                )?;
            let mut seen = BTreeSet::new();
            for definition in &definitions {
                let key = (
                    definition.id.as_str().to_owned(),
                    String::from(definition.version.clone()),
                );
                if !seen.insert(key.clone())
                    || !packages
                        .get(&key)
                        .is_some_and(|package| exact_data(&package.original.definition, definition))
                {
                    return Err(library_error(
                        "Bundled definition list differs from the actual exact dependency ModulePackages",
                    ));
                }
            }
            if seen.len() != packages.len() {
                return Err(library_error(
                    "Bundled dependency ModulePackage is absent from definitions.json",
                ));
            }
        }
        let package = relocate_package(&template, &root)?;
        let key = (
            template.definition.id.as_str().to_owned(),
            String::from(template.definition.version.clone()),
        );
        packages.insert(key, package);
        for package in packages.values() {
            for dependency in &package.package.dependencies {
                if !packages.contains_key(&module_key(dependency))
                    && !self.bundled_modules.contains_key(&module_key(dependency))
                {
                    return Err(error(
                        ErrorCode::MissingReference,
                        "Bundled exact dependency is absent",
                        Some("/dependencies"),
                    ));
                }
            }
            if let Some(existing) = self.bundled_modules.get(&(
                package.package.definition.id.as_str().to_owned(),
                String::from(package.package.definition.version.clone()),
            )) {
                if !exact_data(&existing.original, &package.original)
                    || existing.additions.len() != package.additions.len()
                    || existing
                        .additions
                        .iter()
                        .zip(&package.additions)
                        .any(|(a, b)| !exact_data(&a.entry, &b.entry) || a.bytes != b.bytes)
                {
                    return Err(library_error(
                        "Registered exact bundled version is immutable",
                    ));
                }
            }
        }
        self.bundled_modules.extend(packages);
        Ok(())
    }

    pub(super) fn adoption_source(&self, reference: &ModuleRef) -> Option<&ModulePackage> {
        self.modules.get(&module_key(reference)).or_else(|| {
            self.bundled_modules
                .get(&module_key(reference))
                .map(|entry| &entry.package)
        })
    }

    pub(super) fn bundled_source_path(
        &self,
        reference: &ModuleRef,
        path: &RelativeFilePath,
    ) -> RelativeFilePath {
        if let Some(source) = self.adoption_source(reference) {
            let relocated = format!("{}{}", package_prefix(reference), path.as_str());
            if source
                .files
                .iter()
                .any(|file| file.path.as_str() == relocated)
            {
                return RelativeFilePath::try_from(relocated).unwrap();
            }
        }
        path.clone()
    }

    pub(super) fn module_source_files(&self, reference: &ModuleRef) -> BTreeMap<String, Vec<u8>> {
        let mut files = self
            .backing
            .as_ref()
            .map(|archive| archive.files.clone())
            .unwrap_or_default();
        for addition in &self.additions {
            files.insert(
                addition.entry.file_ref.path.as_str().to_owned(),
                addition.bytes.clone(),
            );
        }
        // Current archived bytes always win over the independent library.
        if let Some(package) = self
            .bundled_modules
            .get(&module_key(reference))
            .filter(|_| !self.modules.contains_key(&module_key(reference)))
        {
            for addition in &package.additions {
                files
                    .entry(addition.entry.file_ref.path.as_str().to_owned())
                    .or_insert_with(|| addition.bytes.clone());
            }
        }
        files
    }

    pub(super) fn stage_library_import(
        &self,
        reference: &ModuleRef,
        candidate: &mut ProjectDraft,
        imports: &mut LibraryImports,
    ) -> Result<(), ApiError> {
        let Some(bundled) = self.bundled_modules.get(&module_key(reference)) else {
            return Ok(());
        };
        let key = module_key(reference);
        if let Some(existing) = self.modules.get(&key) {
            if !exact_data(existing, &bundled.package) {
                return Err(library_error(
                    "Current Project exact version conflicts with bundled source",
                ));
            }
        }
        if imports.modules.contains_key(&key) {
            return Ok(());
        }
        imports.modules.insert(key, bundled.package.clone());
        if !candidate
            .project
            .module_refs
            .iter()
            .any(|actual| module_key(actual) == module_key(reference))
        {
            candidate.project.module_refs.push(reference.clone());
        }
        for dependency in &bundled.package.dependencies {
            self.stage_library_import(dependency, candidate, imports)?;
        }
        for addition in &bundled.additions {
            let path = addition.entry.file_ref.path.as_str();
            let existing = self
                .additions
                .iter()
                .chain(imports.additions.values())
                .find(|actual| {
                    actual
                        .entry
                        .file_ref
                        .path
                        .as_str()
                        .eq_ignore_ascii_case(path)
                });
            if let Some(existing) = existing {
                if existing.bytes != addition.bytes || !exact_data(&existing.entry, &addition.entry)
                {
                    return Err(library_error(
                        "Bundled file conflicts with immutable Project content",
                    ));
                }
                continue;
            }
            if let Some(archive) = &self.backing {
                if let Some(entry) = archive
                    .manifest
                    .documents
                    .iter()
                    .find(|entry| entry.file_ref.path.as_str().eq_ignore_ascii_case(path))
                {
                    if !exact_data(entry, &addition.entry)
                        || archive.files.get(path) != Some(&addition.bytes)
                    {
                        return Err(library_error(
                            "Bundled file conflicts with archived Project content",
                        ));
                    }
                    continue;
                }
            }
            imports.additions.insert(path.to_owned(), addition.clone());
        }
        Ok(())
    }
}
