//! The optional project board (`github-record`, "Issues and visible
//! work"): each reflected todo's issue is an item of the GitHub project
//! `board` names, with a single-select field `Record status` that instrukt
//! owns set to the record's status. The field is created when absent and
//! missing options are added; options are never removed or renamed, and no
//! other field is touched. Every document passes its values as variables.

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
            ... on ProjectV2SingleSelectField { id name options { id name color description } }
          }
        }
      }
    }
  }
}";

const CREATE_FIELD: &str = "mutation($project: ID!, $name: String!, $options: [ProjectV2SingleSelectFieldOptionInput!]!) {
  createProjectV2Field(input: {projectId: $project, dataType: SINGLE_SELECT, name: $name, singleSelectOptions: $options}) {
    projectV2Field { ... on ProjectV2SingleSelectField { id name options { id name color description } } }
  }
}";

const UPDATE_OPTIONS: &str = "mutation($field: ID!, $options: [ProjectV2SingleSelectFieldOptionInput!]!) {
  updateProjectV2Field(input: {fieldId: $field, singleSelectOptions: $options}) {
    projectV2Field { ... on ProjectV2SingleSelectField { id name options { id name color description } } }
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

/// Puts the issue `content` (its node id) on the board with `status`;
/// `item` is its board item when one is known. Returns the item's id.
pub(crate) fn reflect(
    gh: &GitHub,
    board: &BoardRef,
    item: Option<&str>,
    content: &str,
    status: &str,
) -> Result<String, GitHubError> {
    let resolved = gh.graphql(
        RESOLVE,
        &json!({"owner": board.owner, "number": board.number}),
    )?;
    let project = &resolved["repositoryOwner"]["projectV2"];
    let Some(project_id) = project["id"].as_str() else {
        return Err(GitHubError::NotFound {
            path: format!("project {}/{}", board.owner, board.number),
        });
    };
    let existing_field = project["fields"]["nodes"]
        .as_array()
        .and_then(|fields| fields.iter().find(|f| f["name"].as_str() == Some(FIELD)))
        .cloned();
    let field = match existing_field {
        None => gh.graphql(
            CREATE_FIELD,
            &json!({
                "project": project_id,
                "name": FIELD,
                "options": OPTIONS.iter().map(|name| new_option(name)).collect::<Vec<_>>(),
            }),
        )?["createProjectV2Field"]["projectV2Field"]
            .clone(),
        Some(field) => {
            let options = field["options"].as_array().cloned().unwrap_or_default();
            let names: Vec<&str> = options.iter().filter_map(|o| o["name"].as_str()).collect();
            let missing = missing_options(&names);
            if missing.is_empty() {
                field
            } else {
                gh.graphql(
                    UPDATE_OPTIONS,
                    &json!({"field": field["id"], "options": options_with(&options, &missing)}),
                )?["updateProjectV2Field"]["projectV2Field"]
                    .clone()
            }
        }
    };
    let (Some(field_id), Some(option_id)) = (
        field["id"].as_str(),
        field["options"].as_array().and_then(|options| {
            options
                .iter()
                .find(|o| o["name"].as_str() == Some(status))
                .and_then(|o| o["id"].as_str())
        }),
    ) else {
        return Err(GitHubError::Decode {
            path: "/graphql".to_owned(),
            message: format!("the `{FIELD}` field has no `{status}` option"),
        });
    };
    let item_id = match item {
        Some(item) => item.to_owned(),
        None => gh.graphql(
            ADD_ITEM,
            &json!({"project": project_id, "content": content}),
        )?["addProjectV2ItemById"]["item"]["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| GitHubError::Decode {
                path: "/graphql".to_owned(),
                message: "an added board item without an id".to_owned(),
            })?,
    };
    gh.graphql(
        SET_VALUE,
        &json!({"project": project_id, "item": item_id, "field": field_id, "option": option_id}),
    )?;
    Ok(item_id)
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
