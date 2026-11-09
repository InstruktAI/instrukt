//! The automatically reconciled project board (`github-record`, "Issues and visible
//! work"): each reflected todo's issue is an item of the GitHub project
//! `board` names, with a single-select field `Record status` that instrukt
//! owns set to the record's status. The field is created when absent and
//! missing status options are added. The owned Needs you field and cockpit
//! views are reconciled; user fields, views and built-in workflows are preserved.

use instrukt_core::config::BoardRef;
use serde_json::{Value, json};

use super::_github::{GitHub, GitHubError};

/// The field instrukt owns on the board (never the built-in `Status`).
pub(crate) const FIELD: &str = "Record status";

/// Its options: the record's statuses.
pub(crate) const OPTIONS: [&str; 5] = ["active", "iceboxed", "held", "delivered", "withdrawn"];

const RESOLVE: &str = "query($owner: String!, $number: Int!) {
  repositoryOwner(login: $owner) {
    ... on ProjectV2Owner {
      projectV2(number: $number) {
        id
        fields(first: 100) {
          nodes {
            ... on ProjectV2SingleSelectField { id databaseId name options { id name color description } }
          }
        }
      }
    }
  }
}";

const CREATE_FIELD: &str = "mutation($project: ID!, $name: String!, $options: [ProjectV2SingleSelectFieldOptionInput!]!) {
  createProjectV2Field(input: {projectId: $project, dataType: SINGLE_SELECT, name: $name, singleSelectOptions: $options}) {
    projectV2Field { ... on ProjectV2SingleSelectField { id databaseId name options { id name color description } } }
  }
}";

const UPDATE_OPTIONS: &str = "mutation($field: ID!, $options: [ProjectV2SingleSelectFieldOptionInput!]!) {
  updateProjectV2Field(input: {fieldId: $field, singleSelectOptions: $options}) {
    projectV2Field { ... on ProjectV2SingleSelectField { id databaseId name options { id name color description } } }
  }
}";

const ADD_ITEM: &str = "mutation($project: ID!, $content: ID!) {
  addProjectV2ItemById(input: {projectId: $project, contentId: $content}) { item { id } }
}";

const SET_VALUE: &str = "mutation($project: ID!, $item: ID!, $field: ID!, $option: String!) {
  updateProjectV2ItemFieldValue(input: {projectId: $project, itemId: $item, fieldId: $field, value: {singleSelectOptionId: $option}}) {
    projectV2Item { id }
  }
}";

/// The statuses `existing` option names lack, in [`OPTIONS`] order.
pub(crate) fn missing_options(existing: &[&str]) -> Vec<&'static str> {
    OPTIONS
        .iter()
        .copied()
        .filter(|wanted| !existing.contains(wanted))
        .collect()
}

/// The option set an update sends: every existing option as it is (id,
/// name, color, description), then the missing statuses appended.
pub(crate) fn options_with(existing: &[Value], missing: &[&str]) -> Vec<Value> {
    existing
        .iter()
        .map(|o| {
            json!({
                "id": o["id"],
                "name": o["name"],
                "color": o["color"].as_str().unwrap_or("GRAY"),
                "description": o["description"].as_str().unwrap_or(""),
            })
        })
        .chain(missing.iter().map(|name| new_option(name)))
        .collect()
}

fn new_option(name: &str) -> Value {
    json!({"name": name, "color": "GRAY", "description": "instrukt work record status"})
}

const NEEDS_FIELD: &str = "Needs you";
const NEEDS_DESCRIPTION: &str = "Waiting on a person; instrukt will not move it.";
const VIEWS: &str = "query($owner:String!,$number:Int!,$after:String) {
  repositoryOwner(login:$owner) { ... on ProjectV2Owner { projectV2(number:$number) {
    views(first:100,after:$after) { nodes { id name layout filter
      verticalGroupByFields(first:10) { nodes { ... on ProjectV2SingleSelectField { id } } }
    } pageInfo { hasNextPage endCursor } }
  } } }
}";
const CREATE_VIEW: &str = "mutation($project:ID!,$name:String!,$layout:ProjectV2ViewLayout!) {
  createProjectV2View(input:{projectId:$project,name:$name,layout:$layout}) {projectV2View{id}}
}";
const UPDATE_VIEW: &str = "mutation($view:ID!,$layout:ProjectV2ViewLayout!,$filter:String!) {
  updateProjectV2View(input:{viewId:$view,layout:$layout,filter:$filter}) {projectV2View{id}}
}";
const DELETE_VIEW: &str = "mutation($view:ID!) {
  deleteProjectV2View(input:{viewId:$view}) {projectV2View{id}}
}";
const CLEAR_VALUE: &str = "mutation($project:ID!,$item:ID!,$field:ID!) {
  clearProjectV2ItemFieldValue(input:{projectId:$project,itemId:$item,fieldId:$field}) {projectV2Item{id}}
}";

fn unreadable(message: impl Into<String>) -> GitHubError {
    GitHubError::Decode {
        path: "project cockpit".into(),
        message: message.into(),
    }
}
struct Targets {
    project: String,
    status: Value,
    needs: Value,
}

fn field(
    gh: &GitHub,
    project: &str,
    fields: &[Value],
    kind: OwnedField,
) -> Result<Value, GitHubError> {
    let name = kind.name();
    let existing = fields.iter().find(|f| f["name"] == name);
    let options = existing
        .and_then(|f| f["options"].as_array())
        .cloned()
        .unwrap_or_default();
    let target = match kind {
        OwnedField::NeedsYou => {
            let mut option = json!({"name":"Yes","color":"RED","description":NEEDS_DESCRIPTION});
            if let Some(id) = options
                .iter()
                .find(|o| o["name"] == "Yes")
                .and_then(|o| o["id"].as_str())
            {
                option["id"] = id.into();
            }
            vec![option]
        }
        OwnedField::RecordStatus => {
            let names = options
                .iter()
                .filter_map(|o| o["name"].as_str())
                .collect::<Vec<_>>();
            options_with(&options, &missing_options(&names))
        }
    };
    match existing {
        None => Ok(gh.graphql(
            CREATE_FIELD,
            &json!({"project":project,"name":name,"options":target}),
        )?["createProjectV2Field"]["projectV2Field"]
            .clone()),
        Some(existing) if options == target => Ok(existing.clone()),
        Some(existing) => Ok(gh.graphql(
            UPDATE_OPTIONS,
            &json!({"field":existing["id"],"options":target}),
        )?["updateProjectV2Field"]["projectV2Field"]
            .clone()),
    }
}
fn targets(gh: &GitHub, board: &BoardRef) -> Result<Targets, GitHubError> {
    let resolved = gh.graphql(RESOLVE, &json!({"owner":board.owner,"number":board.number}))?;
    let project = &resolved["repositoryOwner"]["projectV2"];
    let id = project["id"]
        .as_str()
        .ok_or_else(|| GitHubError::NotFound {
            path: format!("project {}/{}", board.owner, board.number),
        })?;
    let fields = project["fields"]["nodes"]
        .as_array()
        .ok_or_else(|| unreadable("missing project fields"))?;
    let status = field(gh, id, fields, OwnedField::RecordStatus)?;
    let needs = field(gh, id, fields, OwnedField::NeedsYou)?;
    Ok(Targets {
        project: id.into(),
        status,
        needs,
    })
}
fn option_id<'a>(field: &'a Value, name: &str) -> Result<&'a str, GitHubError> {
    field["options"]
        .as_array()
        .and_then(|options| options.iter().find(|o| o["name"] == name))
        .and_then(|o| o["id"].as_str())
        .ok_or_else(|| {
            unreadable(format!(
                "field {} has no {name} option",
                field["name"].as_str().unwrap_or("unknown")
            ))
        })
}
fn field_id(field: &Value) -> Result<&str, GitHubError> {
    field["id"]
        .as_str()
        .ok_or_else(|| unreadable("field has no node id"))
}

#[derive(Clone, Copy)]
enum OwnedView {
    Board,
    NeedsYou,
}
impl OwnedView {
    fn name(self) -> &'static str {
        match self {
            Self::Board => "Board",
            Self::NeedsYou => NEEDS_FIELD,
        }
    }
    fn layout(self) -> &'static str {
        match self {
            Self::Board => "BOARD_LAYOUT",
            Self::NeedsYou => "TABLE_LAYOUT",
        }
    }
    fn filter(self) -> &'static str {
        match self {
            Self::Board => "",
            Self::NeedsYou => "needs-you:Yes",
        }
    }
    fn matches(self, view: &Value, targets: &Targets) -> bool {
        match self {
            Self::NeedsYou => true,
            Self::Board => view["verticalGroupByFields"]["nodes"]
                .as_array()
                .is_some_and(|fields| fields.len() == 1 && fields[0]["id"] == targets.status["id"]),
        }
    }
    fn create(
        self,
        gh: &GitHub,
        board: &BoardRef,
        targets: &Targets,
    ) -> Result<String, GitHubError> {
        match self {
            Self::Board => {
                let database_id=targets.status["databaseId"].as_u64().ok_or_else(||unreadable("Record status has no REST field id; update GitHub's Projects API capability"))?;
                let created=gh.post_api(&format!("/orgs/{}/projectsV2/{}/views",board.owner,board.number),&json!({"name":self.name(),"layout":"board","filter":self.filter(),"vertical_group_by":[database_id]}))?;
                created["value"]["node_id"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| unreadable("created Board view has no node id"))
            }
            Self::NeedsYou => gh.graphql(
                CREATE_VIEW,
                &json!({"project":targets.project,"name":self.name(),"layout":self.layout()}),
            )?["createProjectV2View"]["projectV2View"]["id"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| unreadable("created view has no node id")),
        }
    }
}
#[derive(Clone, Copy)]
enum OwnedField {
    RecordStatus,
    NeedsYou,
}
impl OwnedField {
    fn name(self) -> &'static str {
        match self {
            Self::RecordStatus => FIELD,
            Self::NeedsYou => NEEDS_FIELD,
        }
    }
}

/// Reconcile the owned fields and views, including on a board with no items.
pub(crate) fn configure(gh: &GitHub, board: &BoardRef) -> Result<(), GitHubError> {
    let targets = targets(gh, board)?;
    let mut views = Vec::new();
    let mut after = Value::Null;
    loop {
        let data = gh.graphql(
            VIEWS,
            &json!({"owner":board.owner,"number":board.number,"after":after}),
        )?;
        let connection = &data["repositoryOwner"]["projectV2"]["views"];
        views.extend(
            connection["nodes"]
                .as_array()
                .ok_or_else(|| unreadable("missing project views"))?
                .iter()
                .cloned(),
        );
        if connection["pageInfo"]["hasNextPage"] != true {
            break;
        }
        let cursor = connection["pageInfo"]["endCursor"].clone();
        if cursor.is_null() || cursor == after {
            return Err(unreadable("view listing has no advancing cursor"));
        }
        after = cursor;
    }
    for kind in [OwnedView::Board, OwnedView::NeedsYou] {
        let owned = views
            .iter()
            .filter(|v| v["name"] == kind.name())
            .collect::<Vec<_>>();
        let matching = owned
            .iter()
            .copied()
            .find(|view| kind.matches(view, &targets));
        let id = match matching {
            Some(view) => view["id"]
                .as_str()
                .ok_or_else(|| unreadable("view has no node id"))?
                .to_owned(),
            None => kind.create(gh, board, &targets)?,
        };
        let layout = kind.layout();
        let filter = kind.filter();
        if matching.is_none_or(|view| {
            view["layout"] != layout || view["filter"].as_str().unwrap_or("") != filter
        }) {
            gh.graphql(
                UPDATE_VIEW,
                &json!({"view":id,"layout":layout,"filter":filter}),
            )?;
        }
        // Column selection is available on REST creation. Keep a correct
        // replacement before removing an owned view with the wrong columns;
        // discovery on retry reconciles a lost creation acknowledgement.
        for duplicate in owned.into_iter().filter(|v| v["id"] != id) {
            let duplicate_id = duplicate["id"]
                .as_str()
                .ok_or_else(|| unreadable("owned view has no node id"))?;
            gh.graphql(DELETE_VIEW, &json!({"view":duplicate_id}))?;
        }
    }
    Ok(())
}

/// Reflect record status and the derived person-wait marker on one issue.
pub(crate) fn reflect(
    gh: &GitHub,
    board: &BoardRef,
    content: &str,
    status: &str,
    needs_you: bool,
) -> Result<String, GitHubError> {
    let targets = targets(gh, board)?;
    let item = gh.graphql(
        ADD_ITEM,
        &json!({"project":targets.project,"content":content}),
    )?["addProjectV2ItemById"]["item"]["id"]
        .as_str()
        .ok_or_else(|| unreadable("added board item has no id"))?
        .to_owned();
    gh.graphql(SET_VALUE,&json!({"project":targets.project,"item":item,"field":field_id(&targets.status)?,"option":option_id(&targets.status,status)?}))?;
    if needs_you {
        gh.graphql(SET_VALUE,&json!({"project":targets.project,"item":item,"field":field_id(&targets.needs)?,"option":option_id(&targets.needs,"Yes")?}))?;
    } else {
        gh.graphql(
            CLEAR_VALUE,
            &json!({"project":targets.project,"item":item,"field":field_id(&targets.needs)?}),
        )?;
    }
    Ok(item)
}

/// Read the configured cockpit or the account's discovery surface without setup.
pub(crate) fn inspect(gh: &GitHub, configured: Option<&BoardRef>) -> Result<(), GitHubError> {
    if let Some(board) = configured {
        let data = gh.graphql(RESOLVE, &json!({"owner":board.owner,"number":board.number}))?;
        let project = &data["repositoryOwner"]["projectV2"];
        if project["id"].as_str().is_none() || !project["fields"]["nodes"].is_array() {
            return Err(unreadable("configured board or its fields are unavailable"));
        }
    } else {
        let owner = gh.repository().split('/').next().unwrap_or_default();
        let data = gh.graphql(
            "query($owner:String!){ repositoryOwner(login:$owner){ id ... on ProjectV2Owner { projectsV2(first:1){nodes{id number title}}}}}",
            &json!({"owner":owner}),
        )?;
        let owner = &data["repositoryOwner"];
        if owner["id"].as_str().is_none() || !owner["projectsV2"]["nodes"].is_array() {
            return Err(unreadable("account project discovery is unavailable"));
        }
    }
    Ok(())
}

/// Discover the repository's cockpit before creating one. Its stable title
/// allows a retry after a lost acknowledgement to find the same resource.
pub(crate) fn setup(gh: &GitHub, configured: Option<&BoardRef>) -> Result<BoardRef, GitHubError> {
    if let Some(board) = configured {
        let data = gh.graphql(RESOLVE, &json!({"owner":board.owner,"number":board.number}))?;
        if data["repositoryOwner"]["projectV2"]["id"]
            .as_str()
            .is_some()
        {
            return Ok(board.clone());
        }
    }
    let owner = gh.repository().split('/').next().unwrap_or_default();
    let title = format!("instrukt: {}", gh.repository());
    let mut after = Value::Null;
    let owner_id = loop {
        let data=gh.graphql("query($owner:String!,$after:String){ repositoryOwner(login:$owner){ id ... on ProjectV2Owner { projectsV2(first:100,after:$after){nodes{id number title} pageInfo{hasNextPage endCursor}}}}}",&json!({"owner":owner,"after":after}))?;
        let node = &data["repositoryOwner"];
        if let Some(number) = node["projectsV2"]["nodes"]
            .as_array()
            .and_then(|nodes| nodes.iter().find(|n| n["title"].as_str() == Some(&title)))
            .and_then(|n| n["number"].as_u64())
        {
            return Ok(BoardRef {
                owner: owner.into(),
                number,
            });
        }
        if node["projectsV2"]["pageInfo"]["hasNextPage"] == true {
            let cursor = node["projectsV2"]["pageInfo"]["endCursor"].clone();
            if cursor.is_null() || cursor == after {
                return Err(GitHubError::Decode {
                    path: "/graphql".into(),
                    message: "project listing has no advancing cursor".into(),
                });
            }
            after = cursor;
        } else {
            break node["id"]
                .as_str()
                .ok_or_else(|| GitHubError::NotFound {
                    path: format!("project owner {owner}"),
                })?
                .to_owned();
        }
    };
    let repository = gh.read("")?;
    let created=gh.graphql("mutation($owner:ID!,$title:String!,$repository:ID!){ createProjectV2(input:{ownerId:$owner,title:$title,repositoryId:$repository}){projectV2{id number}}}",&json!({"owner":owner_id,"title":title,"repository":repository["node_id"]}))?;
    let number = created["createProjectV2"]["projectV2"]["number"]
        .as_u64()
        .ok_or_else(|| GitHubError::Decode {
            path: "/graphql".into(),
            message: "created project has no number".into(),
        })?;
    Ok(BoardRef {
        owner: owner.into(),
        number,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_statuses_the_field_lacks_are_added() {
        assert_eq!(
            missing_options(&[OPTIONS[0], "custom", OPTIONS[3]]),
            vec![OPTIONS[1], OPTIONS[2], OPTIONS[4]]
        );
        assert!(missing_options(&OPTIONS).is_empty());
    }

    #[test]
    fn an_option_update_keeps_every_existing_option_first_and_unchanged() {
        let existing =
            vec![json!({"id": "o1", "name": "custom", "color": "RED", "description": "theirs"})];
        let sent = options_with(&existing, &["active"]);
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0]["id"], existing[0]["id"]);
        assert_eq!(sent[0]["name"], existing[0]["name"]);
        assert_eq!(sent[0]["color"], existing[0]["color"]);
        assert_eq!(sent[1]["name"], OPTIONS[0]);
    }
}
