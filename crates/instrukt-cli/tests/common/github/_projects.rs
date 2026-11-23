//! GitHub projects (v2) over GraphQL: exactly the documents the work
//! record sends.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::_fake::FakeGitHub;
use super::_model::{State, actor};
use super::_serve::Reply;

/// A GitHub project of an owner.
pub(super) struct Project {
    id: String,
    pub(super) owner: String,
    pub(super) number: u64,
    title: String,
    /// Whether the project is public; GitHub creates projects private.
    public: bool,
    pub(super) fields: Vec<ProjectField>,
    items: Vec<ProjectItem>,
    pub(super) views: Vec<Value>,
    workflows: Vec<Value>,
}

impl Project {
    /// A new project as GitHub creates it: private, empty.
    fn new(id: String, owner: String, number: u64, title: String) -> Self {
        Self {
            id,
            owner,
            number,
            title,
            public: false,
            fields: Vec::new(),
            items: Vec::new(),
            views: Vec::new(),
            workflows: Vec::new(),
        }
    }
}

/// A single-select field (other field types are seeded as fields without
/// options and are never served as single-select).
pub(super) struct ProjectField {
    pub(super) id: String,
    name: String,
    single_select: bool,
    options: Vec<FieldOption>,
}

struct FieldOption {
    id: String,
    name: String,
    color: String,
    description: String,
}

struct ProjectItem {
    id: String,
    /// The node id of the issue the item shows.
    content: String,
    /// Field id → option id.
    values: BTreeMap<String, String>,
}

impl FieldOption {
    fn json(&self) -> Value {
        json!({"id": self.id, "name": self.name, "color": self.color, "description": self.description})
    }
}

impl ProjectField {
    fn json(&self) -> Value {
        if self.single_select {
            json!({"id": self.id, "databaseId": field_database_id(&self.id), "name": self.name, "options": self.options.iter().map(FieldOption::json).collect::<Vec<_>>()})
        } else {
            // Not a single-select field: the inline fragment selects nothing.
            json!({})
        }
    }
}

impl FakeGitHub {
    /// The project numbers visible on an account.
    pub fn project_numbers(&self, owner: &str) -> Vec<u64> {
        self.lock()
            .projects
            .iter()
            .filter(|project| project.owner == owner)
            .map(|project| project.number)
            .collect()
    }

    /// Sets account project-discovery availability at the API boundary.
    pub fn set_project_discovery_available(&self, available: bool) {
        self.lock().project_discovery_available = available;
    }

    /// Adds the project `<owner>/<number>`.
    pub fn add_project(&self, owner: &str, number: u64) -> &Self {
        let mut state = self.lock();
        let id = format!("PVT_{}", state.next_id());
        state
            .projects
            .push(Project::new(id, owner.to_owned(), number, String::new()));
        self
    }

    /// Whether project `<owner>/<number>` is public.
    pub fn project_public(&self, owner: &str, number: u64) -> bool {
        self.lock()
            .projects
            .iter()
            .find(|p| p.owner == owner && p.number == number)
            .expect("project exists")
            .public
    }

    /// Changes project `<owner>/<number>`'s visibility, as a maintainer
    /// does in the project's settings.
    pub fn set_project_public(&self, owner: &str, number: u64, public: bool) {
        self.lock()
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .expect("project exists")
            .public = public;
    }

    /// Makes GitHub refuse this viewer's project visibility changes, as an
    /// organization that reserves them for its owners does.
    pub fn set_project_visibility_refused(&self, refused: bool) {
        self.lock().project_visibility_refused = refused;
    }

    /// Adds a field to project `<owner>/<number>`: a single-select field
    /// with `options`, or, with `None`, a field of another type.
    pub fn add_project_field(
        &self,
        owner: &str,
        number: u64,
        name: &str,
        options: Option<&[&str]>,
    ) {
        let mut state = self.lock();
        let field_id = format!("PVTF_{}", state.next_id());
        let mut made = Vec::new();
        for option in options.unwrap_or_default() {
            made.push(FieldOption {
                id: format!("OPT_{}", state.next_id()),
                name: (*option).to_owned(),
                color: "BLUE".into(),
                description: String::new(),
            });
        }
        let project = state
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap();
        project.fields.push(ProjectField {
            id: field_id,
            name: name.to_owned(),
            single_select: options.is_some(),
            options: made,
        });
    }

    pub fn remove_project_field(&self, owner: &str, number: u64, name: &str) {
        let mut state = self.lock();
        state
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .fields
            .retain(|f| f.name != name);
    }
    pub fn clear_project_items(&self, owner: &str, number: u64) {
        let mut state = self.lock();
        state
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .items
            .clear();
    }
    pub fn seed_project_view(
        &self,
        owner: &str,
        number: u64,
        name: &str,
        layout: &str,
        filter: &str,
        columns: &[&str],
    ) {
        let mut state = self.lock();
        let id = state.next_id();
        let project = state
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap();
        let fields = columns
            .iter()
            .map(|name| {
                field_database_id(&project.fields.iter().find(|f| f.name == *name).unwrap().id)
            })
            .collect::<Vec<_>>();
        let number = project
            .views
            .iter()
            .filter_map(|v| v["number"].as_u64())
            .max()
            .unwrap_or(0)
            + 1;
        project.views.push(json!({"id":id,"node_id":format!("PVTV_{id}"),"number":number,"name":name,"layout":layout,"filter":filter,"vertical_group_by":fields}));
    }
    pub fn edit_project_view(&self, owner: &str, number: u64, name: &str, patch: &Value) {
        let mut state = self.lock();
        let view = state
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .views
            .iter_mut()
            .find(|v| v["name"] == name)
            .unwrap();
        for (key, value) in patch.as_object().unwrap() {
            view[key] = value.clone();
        }
    }
    pub fn remove_project_view(&self, owner: &str, number: u64, name: &str) {
        self.lock()
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .views
            .retain(|v| v["name"] != name);
    }
    pub fn project_views(&self, owner: &str, number: u64) -> Vec<Value> {
        self.lock()
            .projects
            .iter()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .views
            .clone()
    }
    pub fn project_field_details(&self, owner: &str, number: u64) -> Vec<Value> {
        self.lock()
            .projects
            .iter()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .fields
            .iter()
            .map(ProjectField::json)
            .collect()
    }
    pub fn drift_project_option(&self, owner: &str, number: u64, name: &str) {
        let mut state = self.lock();
        let field = state
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .fields
            .iter_mut()
            .find(|f| f.name == name)
            .unwrap();
        for option in &mut field.options {
            option.color = "BLUE".into();
            option.description = "drifted".into();
        }
    }
    pub fn seed_project_workflow(&self, owner: &str, number: u64, workflow: Value) {
        self.lock()
            .projects
            .iter_mut()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .workflows
            .push(workflow);
    }
    pub fn project_workflows(&self, owner: &str, number: u64) -> Vec<Value> {
        self.lock()
            .projects
            .iter()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap()
            .workflows
            .clone()
    }
    /// A project's fields, each with its option names, in order.
    pub fn project_fields(&self, owner: &str, number: u64) -> Vec<(String, Vec<String>)> {
        let state = self.lock();
        let project = state
            .projects
            .iter()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap();
        project
            .fields
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    f.options.iter().map(|o| o.name.clone()).collect(),
                )
            })
            .collect()
    }

    /// A project's items: the issue node id each shows, and each set field
    /// as (field name, option name).
    pub fn project_items(&self, owner: &str, number: u64) -> Vec<(String, Vec<(String, String)>)> {
        let state = self.lock();
        let project = state
            .projects
            .iter()
            .find(|p| p.owner == owner && p.number == number)
            .unwrap();
        project
            .items
            .iter()
            .map(|item| {
                let values = item
                    .values
                    .iter()
                    .filter_map(|(field, option)| {
                        let field = project.fields.iter().find(|f| &f.id == field)?;
                        let option = field.options.iter().find(|o| &o.id == option)?;
                        Some((field.name.clone(), option.name.clone()))
                    })
                    .collect();
                (item.content.clone(), values)
            })
            .collect()
    }
}

fn graphql_error(message: &str) -> Reply {
    Reply::ok(json!({"data": null, "errors": [{"message": message}]}))
}

fn field_payload(field: &ProjectField) -> Value {
    field.json()
}

/// The value `query` gives `field` in the `input` object of its
/// `mutation(input:{...})` call, as GitHub reads it: a `$variable` resolved
/// from `vars`, or a literal. `None` when that input does not set the field.
fn input_field(query: &str, mutation: &str, field: &str, vars: &Value) -> Option<Value> {
    let call = &query[query.find(&format!("{mutation}("))?..];
    let input = &call[call.find('{')? + 1..];
    let input = &input[..input.find('}')?];
    input.split(',').find_map(|entry| {
        let (name, value) = entry.split_once(':')?;
        if name.trim() != field {
            return None;
        }
        let value = value.trim();
        match value.strip_prefix('$') {
            Some(variable) => Some(vars[variable].clone()),
            None => serde_json::from_str(value).ok(),
        }
    })
}

/// Serves the GraphQL documents the work record sends; any other document
/// is an error.
pub(super) fn graphql(state: &mut State, input: &Value) -> Reply {
    let query = input["query"].as_str().unwrap_or_default();
    let vars = &input["variables"];
    if query.contains("viewer { id login databaseId }") {
        let bot = actor(&state.login);
        return Reply::ok(
            json!({"data":{"viewer":{"id":bot["node_id"],"databaseId":bot["id"],"login":bot["login"]}}}),
        );
    }
    if query.contains("projectsV2(first:") {
        if !state.project_discovery_available {
            return graphql_error("project discovery is unavailable");
        }
        let owner = vars["owner"].as_str().unwrap();
        let nodes = state
            .projects
            .iter()
            .filter(|p| p.owner == owner)
            .map(|p| json!({"id":p.id,"number":p.number,"title":p.title}))
            .collect::<Vec<_>>();
        return Reply::ok(
            json!({"data":{"repositoryOwner":{"id":format!("OWNER_{owner}"),"projectsV2":{"nodes":nodes,"pageInfo":{"hasNextPage":false}}}}}),
        );
    }
    if query.contains("createProjectV2(input:") {
        let has_repo_access = state
            .active_token
            .as_deref()
            .and_then(|t| state.tokens.get(t))
            .is_none_or(|(_, _, has_repo)| *has_repo);
        if query.contains("repositoryId") && !has_repo_access {
            return graphql_error("Resource not accessible by integration");
        }
        let owner = vars["owner"]
            .as_str()
            .unwrap()
            .trim_start_matches("OWNER_")
            .to_owned();
        let number = state
            .projects
            .iter()
            .filter(|p| p.owner == owner)
            .map(|p| p.number)
            .max()
            .unwrap_or(0)
            + 1;
        let id = format!("PVT_{}", state.next_id());
        state.projects.push(Project::new(
            id.clone(),
            owner,
            number,
            vars["title"].as_str().unwrap().into(),
        ));
        return Reply::ok(
            json!({"data":{"createProjectV2":{"projectV2":{"id":id,"number":number}}}}),
        );
    }
    if query.contains("updateProjectV2(input:") {
        let public = input_field(query, "updateProjectV2", "public", vars);
        if public.is_some() && state.project_visibility_refused {
            // What GitHub was observed to answer an App the organization's
            // visibility policy does not admit.
            return Reply::ok(json!({"data": {"updateProjectV2": null}, "errors": [{
                "type": "FORBIDDEN",
                "path": ["updateProjectV2"],
                "message": "Viewer not authorized to change project visibility",
            }]}));
        }
        let Some(project) = state
            .projects
            .iter_mut()
            .find(|p| vars["project"] == p.id.as_str())
        else {
            return graphql_error("no such project");
        };
        if let Some(public) = public.as_ref().and_then(Value::as_bool) {
            project.public = public;
        }
        return Reply::ok(json!({"data": {"updateProjectV2": {"projectV2": {"id": project.id}}}}));
    }
    if query.contains("views(first:") {
        let project = state
            .projects
            .iter()
            .find(|p| {
                p.owner == vars["owner"].as_str().unwrap()
                    && Some(p.number) == vars["number"].as_u64()
            })
            .unwrap();
        let offset = vars["after"]
            .as_str()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        let nodes=project.views.iter().skip(offset).take(100).map(|v| {
            let fields=v["vertical_group_by"].as_array().unwrap().iter().filter_map(|id|project.fields.iter().find(|f|Some(field_database_id(&f.id))==id.as_u64())).map(|f|json!({"id":f.id})).collect::<Vec<_>>();
            json!({"id":v["node_id"],"name":v["name"],"number":v["number"],"filter":v["filter"],"layout":match v["layout"].as_str().unwrap(){"board"=>"BOARD_LAYOUT","table"=>"TABLE_LAYOUT",_=>"ROADMAP_LAYOUT"},"verticalGroupByFields":{"nodes":fields}})
        }).collect::<Vec<_>>();
        let more = offset + 100 < project.views.len();
        return Reply::ok(
            json!({"data":{"repositoryOwner":{"projectV2":{"id":project.id,"views":{"nodes":nodes,"pageInfo":{"hasNextPage":more,"endCursor":(offset+100).to_string()}}}}}}),
        );
    }
    if query.contains("createProjectV2View(") {
        let id = state.next_id();
        let Some(project) = state
            .projects
            .iter_mut()
            .find(|p| p.id == vars["project"].as_str().unwrap())
        else {
            return graphql_error("no such project");
        };
        let number = project
            .views
            .iter()
            .filter_map(|v| v["number"].as_u64())
            .max()
            .unwrap_or(0)
            + 1;
        let view = json!({"id":id,"node_id":format!("PVTV_{id}"),"number":number,"name":vars["name"],"layout":if vars["layout"]=="BOARD_LAYOUT"{"board"}else{"table"},"filter":"","vertical_group_by":[]});
        project.views.push(view.clone());
        return Reply::ok(
            json!({"data":{"createProjectV2View":{"projectV2View":{"id":view["node_id"]}}}}),
        );
    }
    if query.contains("updateProjectV2View(") {
        let Some(view) = state
            .projects
            .iter_mut()
            .flat_map(|p| &mut p.views)
            .find(|v| v["node_id"] == vars["view"])
        else {
            return graphql_error("no such view");
        };
        if let Some(layout) = vars.get("layout") {
            view["layout"] = if layout == "BOARD_LAYOUT" {
                "board"
            } else {
                "table"
            }
            .into();
        }
        if let Some(filter) = vars.get("filter") {
            view["filter"] = filter.clone();
        }
        return Reply::ok(
            json!({"data":{"updateProjectV2View":{"projectV2View":{"id":view["node_id"]}}}}),
        );
    }
    if query.contains("deleteProjectV2View(") {
        for project in &mut state.projects {
            project.views.retain(|v| v["node_id"] != vars["view"]);
        }
        return Reply::ok(
            json!({"data":{"deleteProjectV2View":{"projectV2View":{"id":vars["view"]}}}}),
        );
    }
    if query.contains("deleteProjectV2Workflow(") {
        for project in &mut state.projects {
            project.workflows.retain(|v| v["id"] != vars["workflow"]);
        }
        return Reply::ok(
            json!({"data":{"deleteProjectV2Workflow":{"deletedWorkflowId":vars["workflow"]}}}),
        );
    }
    if query.contains("repositoryOwner") {
        let owner = vars["owner"].as_str().unwrap_or_default();
        let number = vars["number"].as_u64().unwrap_or_default();
        let project = state
            .projects
            .iter()
            .find(|p| p.owner == owner && p.number == number)
            .map(|p| {
                json!({"id": p.id, "fields": {"nodes": p.fields.iter().map(ProjectField::json).collect::<Vec<_>>()}})
            });
        return Reply::ok(json!({"data": {"repositoryOwner": {"projectV2": project}}}));
    }
    if query.contains("createProjectV2Field") {
        let field_id = format!("PVTF_{}", state.next_id());
        let mut options = Vec::new();
        for option in vars["options"].as_array().cloned().unwrap_or_default() {
            options.push(FieldOption {
                id: format!("OPT_{}", state.next_id()),
                name: option["name"].as_str().unwrap_or_default().to_owned(),
                color: option["color"].as_str().unwrap_or("GRAY").to_owned(),
                description: option["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
        let Some(project) = state
            .projects
            .iter_mut()
            .find(|p| vars["project"] == p.id.as_str())
        else {
            return graphql_error("no such project");
        };
        project.fields.push(ProjectField {
            id: field_id,
            name: vars["name"].as_str().unwrap_or_default().to_owned(),
            single_select: true,
            options,
        });
        let payload = project.fields.last().map(field_payload).unwrap_or_default();
        return Reply::ok(json!({"data": {"createProjectV2Field": {"projectV2Field": payload}}}));
    }
    if query.contains("updateProjectV2Field(") {
        let inputs = vars["options"].as_array().cloned().unwrap_or_default();
        let mut fresh = Vec::new();
        for _ in &inputs {
            fresh.push(format!("OPT_{}", state.next_id()));
        }
        let field = state
            .projects
            .iter_mut()
            .flat_map(|p| p.fields.iter_mut())
            .find(|f| vars["field"] == f.id.as_str());
        let Some(field) = field else {
            return graphql_error("no such field");
        };
        // GitHub replaces the option set; an option sent with its id keeps
        // it.
        field.options = inputs
            .iter()
            .zip(fresh)
            .map(|(option, fresh)| FieldOption {
                id: option["id"].as_str().map_or(fresh, str::to_owned),
                name: option["name"].as_str().unwrap_or_default().to_owned(),
                color: option["color"].as_str().unwrap_or("GRAY").to_owned(),
                description: option["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            })
            .collect();
        let payload = field_payload(field);
        return Reply::ok(json!({"data": {"updateProjectV2Field": {"projectV2Field": payload}}}));
    }
    if query.contains("addProjectV2ItemById") {
        let item_id = format!("PVTI_{}", state.next_id());
        let content = vars["content"].as_str().unwrap_or_default().to_owned();
        let Some(project) = state
            .projects
            .iter_mut()
            .find(|p| vars["project"] == p.id.as_str())
        else {
            return graphql_error("no such project");
        };
        let id = match project.items.iter().find(|i| i.content == content) {
            Some(existing) => existing.id.clone(),
            None => {
                project.items.push(ProjectItem {
                    id: item_id.clone(),
                    content,
                    values: BTreeMap::new(),
                });
                item_id
            }
        };
        return Reply::ok(json!({"data": {"addProjectV2ItemById": {"item": {"id": id}}}}));
    }
    if query.contains("clearProjectV2ItemFieldValue(") {
        let Some(item) = state
            .projects
            .iter_mut()
            .find(|p| p.id == vars["project"].as_str().unwrap())
            .and_then(|p| {
                p.items
                    .iter_mut()
                    .find(|i| i.id == vars["item"].as_str().unwrap())
            })
        else {
            return graphql_error("no such item");
        };
        item.values.remove(vars["field"].as_str().unwrap());
        return Reply::ok(
            json!({"data":{"clearProjectV2ItemFieldValue":{"projectV2Item":{"id":item.id}}}}),
        );
    }
    if query.contains("updateProjectV2ItemFieldValue") {
        let Some(project) = state
            .projects
            .iter_mut()
            .find(|p| vars["project"] == p.id.as_str())
        else {
            return graphql_error("no such project");
        };
        let Some(item) = project
            .items
            .iter_mut()
            .find(|i| vars["item"] == i.id.as_str())
        else {
            return graphql_error("no such item");
        };
        item.values.insert(
            vars["field"].as_str().unwrap_or_default().to_owned(),
            vars["option"].as_str().unwrap_or_default().to_owned(),
        );
        let id = item.id.clone();
        return Reply::ok(
            json!({"data": {"updateProjectV2ItemFieldValue": {"projectV2Item": {"id": id}}}}),
        );
    }
    graphql_error("the fake GitHub does not serve this document")
}

pub(super) fn field_database_id(id: &str) -> u64 {
    id.rsplit('_').next().unwrap().parse().unwrap()
}
