//! Publication's read-only cockpit: sections fail independently and never probe a runtime.
use super::{PublishedRecord, position_owners};
use crate::{
    config::{
        self, Configuration, Env, Feature, Layer, LayerInspection, Module, Runtime, Unavailable,
        WorkflowCapability,
    },
    git,
    home::Home,
    ids::SessionRef,
    repository,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct Request<'a> {
    pub home: &'a Home,
    pub cwd: &'a Path,
    pub version: &'a str,
    pub executable: Result<PathBuf, std::io::Error>,
    pub caller_error: Option<String>,
    pub env: &'a Env,
}
#[derive(Serialize)]
pub struct Report {
    pub instrukt: Value,
    pub machine: Value,
    pub runtime: Value,
    pub modules: Value,
    pub repository: Value,
    pub published: Vec<Value>,
    pub projects: Vec<Value>,
    #[serde(skip)]
    failed: bool,
}
impl Report {
    pub fn healthy(&self) -> bool {
        !self.failed
    }
}
fn error(path: &Path, reason: impl std::fmt::Display) -> Value {
    json!({"path":path,"reason":reason.to_string()})
}
fn layer(home: &Home, layer: &Layer, errors: &mut Vec<Value>) -> Option<LayerInspection> {
    match config::inspect_layer(home, layer) {
        Ok(view) => Some(view),
        Err(reason) => {
            errors.push(error(&layer.path(home), reason));
            None
        }
    }
}
/// Report only recorded/local facts. No transport, publication, lock or write is performed.
pub fn inspect(request: &Request<'_>) -> Report {
    let home = request.home.for_inspection();
    let mut machine_errors = Vec::new();
    let global = layer(&home, &Layer::Global, &mut machine_errors);
    let findings = if global.as_ref().is_some_and(|v| v.present) {
        match config::validate_local(&home,&Layer::Global) {Ok(findings)=>json!(findings.into_iter().map(|f|json!({"document":f.document,"key":f.key,"kind":format!("{:?}",f.kind),"message":f.message})).collect::<Vec<_>>()),Err(reason)=>{machine_errors.push(error(&home.global_config(),reason));json!([])}}
    } else {
        json!([])
    };
    let configured = match home.global_config().try_exists() {
        Ok(value) => Some(value),
        Err(reason) => {
            machine_errors.push(error(&home.global_config(), reason));
            None
        }
    };
    let selection_migration = match config::Agents::inspection_notice(&home) {
        Ok(notice) => notice,
        Err(reason) => {
            machine_errors.push(error(&home.agents_file(), reason));
            None
        }
    };
    let machine = json!({"configured":configured,"path":home.global_config(),"schema_version":global.as_ref().and_then(|v|v.schema_version),"migration":global.as_ref().and_then(|v|v.migration.clone()),"findings":findings,"selection_migration":selection_migration,"errors":machine_errors});
    let mut runtime_errors = Vec::new();
    if let Some(reason) = &request.caller_error {
        runtime_errors.push(error(home.root(), reason));
    }
    let (selected, runtime_unreadable) =
        match Configuration::load_global(&home).and_then(|c| c.runner_runtime()) {
            Ok(selected) => (selected, false),
            Err(reason) => {
                runtime_errors.push(error(&home.global_config(), reason));
                (None, true)
            }
        };
    let caller = selected.and_then(|r| r.caller(request.env));
    let runtime = runtime_section(selected, caller, &runtime_errors);
    let mut repository_errors = Vec::new();
    let root = match git::worktree_root(request.cwd) {
        Ok(root) => root,
        Err(reason) => {
            repository_errors.push(error(request.cwd, reason));
            None
        }
    };
    let project_layer = root
        .as_ref()
        .and_then(|root| layer(&home, &Layer::Project(root.clone()), &mut repository_errors));
    let project = project_layer.as_ref().and_then(|v| match v.project() {
        Ok(project) => project,
        Err(reason) => {
            repository_errors.push(error(&v.path, reason));
            None
        }
    });
    let modules_unreadable = global.is_none()
        || (root.is_some() && project_layer.is_none())
        || (root.is_none() && !repository_errors.is_empty());
    let mut modules_errors = Vec::new();
    let mut modules = serde_json::Map::new();
    for module in Module::ALL {
        let setting = |view: Option<&LayerInspection>| {
            view.map(|v| v.module_setting(module))
                .transpose()
                .map(Option::flatten)
        };
        let value = if modules_unreadable {
            modules_errors.push(error(
                request.cwd,
                format!(
                    "cannot determine modules.{} until the configuration layers can be read",
                    module.key()
                ),
            ));
            json!({"enabled":null,"layer":null})
        } else {
            match setting(project_layer.as_ref()).and_then(|project| {
                Ok(match project {
                    Some(value) => (value, "project"),
                    None => match setting(global.as_ref())? {
                        Some(value) => (value, "global"),
                        None => (module == Module::History, "default"),
                    },
                })
            }) {
                Ok((enabled, layer)) => json!({"enabled":enabled,"layer":layer}),
                Err(reason) => {
                    modules_errors.push(error(request.cwd, reason));
                    json!({"enabled":null,"layer":null})
                }
            }
        };
        modules.insert(module.key().into(), value);
    }
    modules.insert("features".into(), features(selected, runtime_unreadable));
    modules.insert("errors".into(), json!(modules_errors));
    let mut publication_errors = Vec::new();
    let records = match PublishedRecord::load_all(&home) {
        Ok(records) => records,
        Err(reason) => {
            publication_errors.push(error(&home.publication_state_dir(), reason));
            Vec::new()
        }
    };
    let owners = match position_owners(&home) {
        Ok(owners) => owners,
        Err(reason) => {
            publication_errors.push(error(&home.claims_dir(), reason));
            BTreeMap::new()
        }
    };
    let mut published = Vec::new();
    for (domain, owner) in &owners {
        let record = records.iter().find(|record| &record.project == owner);
        let index = home.domain_dir(domain).join("index.yaml");
        let mut errors = Vec::new();
        let description = if domain.as_str() == "general" {
            None
        } else {
            match std::fs::read(&index) {
                Ok(bytes) => match serde_yaml_ng::from_slice::<crate::docs::DomainIndex>(&bytes) {
                    Ok(index) => index.description,
                    Err(reason) => {
                        errors.push(error(&index, reason));
                        None
                    }
                },
                Err(reason) if reason.kind() == std::io::ErrorKind::NotFound => None,
                Err(reason) => {
                    errors.push(error(&index, reason));
                    None
                }
            }
        };
        if record.is_none() {
            errors.push(error(
                &home.publication_state_dir(),
                format!("owner {owner} has no publication record"),
            ));
        }
        published.push(json!({"position":domain,"owner":owner,"checkout":record.map(|r|&r.checkout),"commit":record.map(|r|&r.commit),"description":description,"errors":errors}));
    }
    if !publication_errors.is_empty() {
        published.push(json!({"errors":publication_errors}));
    }
    let repository=root.as_ref().map(|root| {
        let hooks=match repository::hooks_status(&home,root) {Ok(hooks)=>Some(hooks),Err(reason)=>{repository_errors.push(error(root,reason));None}};
        let hooks_json=hooks.as_ref().map(|hooks|hooks.hooks.iter().map(|hook|json!({"hook":hook.hook.name(),"path":hook.file,"current":hook.section==repository::SectionStatus::Current,"executable":!hook.not_executable})).collect::<Vec<_>>()).unwrap_or_default();
        let installed=hooks.as_ref().is_some_and(|h|!h.hooks.is_empty()&&h.hooks.iter().all(|h|h.section==repository::SectionStatus::Current&&!h.not_executable));
        let adoption=if hooks.is_none(){"unknown"}else if project.is_some()&&installed {"adopted"} else if project_layer.as_ref().is_some_and(|v|v.present)||installed {"partly_adopted"} else {"not_adopted"};
        let code=match Configuration::load(&home,Some(root)).and_then(|c|{let api=c.github_api_url()?.to_owned();crate::github::code_host(&c,&api,root).map_err(|e|config::ConfigError::InvalidValue {key:"github".into(),message:e.to_string()})}) {
            Ok(crate::github::CodeHost::GitHub {repository})=>json!({"repository":repository,"on_github":true}),Ok(crate::github::CodeHost::Elsewhere {reason})=>json!({"repository":null,"on_github":false,"reason":reason}),Err(reason)=>{repository_errors.push(error(root,reason));Value::Null}
        };
        let record=records.iter().find(|r|Some(&r.project)==project.as_ref());
        let main_ahead=record.map(|record|super::_source::canonical_main(root).and_then(|(_,head)|git::succeeds(root,["merge-base","--is-ancestor",record.commit.as_str(),head.as_str()]).map(|ancestor|ancestor&&head!=record.commit).map_err(super::PublicationError::from))).transpose();
        let main_ahead=match main_ahead {Ok(value)=>value,Err(reason)=>{repository_errors.push(error(root,reason));None}};
        let positions=match positions(root,&owners,project.as_ref()){Ok(positions)=>positions,Err(reason)=>{repository_errors.push(error(root,reason));vec![]}};
        json!({"path":root,"project_layer":root.join(config::CONFIG_FILE_NAME).try_exists().ok(),"migration":project_layer.as_ref().and_then(|v|v.migration.as_ref()),"project":project,"adoption":adoption,"hooks":hooks_json,"code":code,"positions":positions,"published_commit":record.map(|r|&r.commit),"main_ahead":main_ahead,"errors":repository_errors})
    }).unwrap_or_else(||if repository_errors.is_empty(){Value::Null}else{json!({"errors":repository_errors})});
    let mut projects = records
        .iter()
        .filter(|r| Some(&r.project) != project.as_ref())
        .map(|r| json!({"project":r.project,"checkout":r.checkout,"commit":r.commit}))
        .collect::<Vec<_>>();
    if !publication_errors.is_empty() {
        projects.push(json!({"errors":publication_errors}));
    }
    let failed = request.executable.is_err()
        || !machine_errors.is_empty()
        || !runtime_errors.is_empty()
        || !modules_errors.is_empty()
        || !repository_errors.is_empty()
        || published
            .iter()
            .any(|p| p["errors"].as_array().is_some_and(|a| !a.is_empty()));
    Report {
        instrukt: json!({"version":request.version,"path":request.executable.as_ref().ok(),"errors":request.executable.as_ref().err().map(|e|vec![json!({"path":null,"reason":format!("cannot locate the running binary: {e}")})]).unwrap_or_default()}),
        machine,
        runtime,
        modules: Value::Object(modules),
        repository,
        published,
        projects,
        failed,
    }
}
fn runtime_section(
    runtime: Option<Runtime>,
    caller: Option<SessionRef>,
    errors: &[Value],
) -> Value {
    let capabilities=runtime.map(|runtime| {
        let native=runtime.native_capabilities();
        let workflow=[WorkflowCapability::WorktreeCreate,WorkflowCapability::SharedDelegation,WorkflowCapability::WorktreeAttach,WorkflowCapability::Link,WorkflowCapability::Notify,WorkflowCapability::AgentSelection].into_iter().map(|c|(c.to_string(),c.supported_by(runtime))).collect::<BTreeMap<_,_>>();
        json!({"native":{"shared_delegation":native.shared_delegation,"worktree_attach":native.worktree_attach,"agent_selection":native.agent_selection},"workflow":workflow})
    });
    json!({"selected":runtime.map(|r|r.name()),"caller":caller,"capabilities":capabilities,"errors":errors})
}
fn features(runtime: Option<Runtime>, unreadable: bool) -> Value {
    let mut result = serde_json::Map::new();
    for (key, feature) in [
        ("standing_agents", Feature::StandingAgents),
        ("history_extraction", Feature::HistoryExtraction),
        ("todo_dispatch", Feature::TodoDispatch),
        ("recurring_tasks", Feature::RecurringTasks),
        ("anomaly_notification", Feature::AnomalyNotification),
        ("session_status", Feature::SessionStatus),
        ("linked_session_input", Feature::LinkedSessionInput),
        ("worktree_attachment", Feature::WorktreeAttachment),
    ] {
        if unreadable {
            result.insert(key.into(),json!({"available":null,"missing":[],"errors":["runtime configuration could not be read"]}));
            continue;
        }
        let unavailable = config::feature_availability(runtime, &[feature]);
        let missing = match &unavailable {
            None => vec![],
            Some(Unavailable::NoRuntime) => vec!["no runtime chosen"],
            Some(Unavailable::MissingCapabilities { capabilities, .. }) => capabilities.clone(),
        };
        result.insert(
            key.into(),
            json!({"available":unavailable.is_none(),"missing":missing}),
        );
    }
    Value::Object(result)
}
fn positions(
    root: &Path,
    owners: &BTreeMap<crate::ids::DomainName, crate::ids::ProjectSlug>,
    project: Option<&crate::ids::ProjectSlug>,
) -> Result<Vec<Value>, super::PublicationError> {
    let mut candidates = vec![("general".to_owned(), root.join("docs/general"))];
    let domains = root.join("docs/domain");
    match std::fs::read_dir(&domains) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|source| super::PublicationError::Io {
                    path: domains.clone(),
                    source,
                })?;
                candidates.push((
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(super::PublicationError::Io {
                path: domains,
                source,
            });
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.push(("project".into(), root.join("docs/project")));
    let mut result = Vec::new();
    for (name, path) in candidates {
        if !super::_adoption::holds_files(&path)? {
            continue;
        }
        let owner = crate::ids::DomainName::parse(&name)
            .ok()
            .and_then(|d| owners.get(&d));
        result.push(json!({"position":name,"path":path,"owner":owner,"conflict":owner.is_some()&&owner!=project}));
    }
    Ok(result)
}
