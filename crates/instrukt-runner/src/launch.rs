//! A procedure launch (`session-continuity`, "Runtime worktree binding";
//! `snippet-authoring-schema`, `parameters`): the procedure named for a new
//! session, its arguments bound to its declared parameters, and the
//! opening message the session starts with.

use instrukt_core::home::Home;
use instrukt_core::ids::{RoleName, SnippetId};
use instrukt_core::index::{GlobalIndex, IndexDispatch, IndexEntry, IndexError, IndexParameter};

/// A launch was refused before any session existed.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("no procedure `{0}` is published; check `instrukt docs index`")]
    UnknownProcedure(SnippetId),
    #[error("`{0}` is not a procedure")]
    NotAProcedure(SnippetId),
    #[error("`{procedure}` requires the argument `{name}`")]
    MissingArgument { procedure: SnippetId, name: String },
    #[error("`{procedure}` takes no argument for `{surplus}`")]
    SurplusArguments {
        procedure: SnippetId,
        surplus: String,
    },
    #[error(transparent)]
    Index(#[from] IndexError),
}

/// A procedure ready to start a session with.
#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub procedure: SnippetId,
    /// The role the session carries; `None` when the procedure declares
    /// none.
    pub role: Option<RoleName>,
    pub dispatch: Option<IndexDispatch>,
    /// The session's opening message.
    pub message: String,
}

/// The published procedure `id`, from the home's index.
pub fn procedure(home: &Home, id: &SnippetId) -> Result<IndexEntry, LaunchError> {
    let entry = GlobalIndex::load(home)?
        .and_then(|index| index.snippets.into_iter().find(|entry| &entry.id == id))
        .ok_or_else(|| LaunchError::UnknownProcedure(id.clone()))?;
    if entry.taxonomy != "procedure" {
        return Err(LaunchError::NotAProcedure(id.clone()));
    }
    Ok(entry)
}

/// Prepares the launch of `entry` with the raw argument text `args` and
/// the additional briefing `brief`.
pub fn prepare(
    entry: &IndexEntry,
    args: Option<&str>,
    brief: Option<&str>,
) -> Result<Launch, LaunchError> {
    let raw = args.unwrap_or("");
    let bound = bind_arguments(&entry.id, &entry.parameters, raw)?;
    Ok(Launch {
        procedure: entry.id.clone(),
        role: entry
            .role
            .as_deref()
            .and_then(|role| RoleName::parse(role).ok()),
        dispatch: entry.dispatch.clone(),
        message: compose(&entry.id, &bound, raw, brief),
    })
}

/// Binds `raw` to `parameters`: one whitespace token per positional entry
/// in order, the untokenized remainder to a final `rest` entry. A missing
/// required argument or surplus tokens without a `rest` entry refuse.
pub fn bind_arguments(
    procedure: &SnippetId,
    parameters: &[IndexParameter],
    raw: &str,
) -> Result<Vec<(String, String)>, LaunchError> {
    let mut bound = Vec::new();
    let mut rest = raw.trim_start();
    for parameter in parameters {
        if parameter.rest {
            if !rest.trim().is_empty() {
                bound.push((parameter.name.clone(), rest.trim_end().to_owned()));
            } else if parameter.required {
                return Err(missing(procedure, parameter));
            }
            rest = "";
            break;
        }
        let token_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let token = &rest[..token_end];
        if token.is_empty() {
            if parameter.required {
                return Err(missing(procedure, parameter));
            }
            continue;
        }
        bound.push((parameter.name.clone(), token.to_owned()));
        rest = rest[token_end..].trim_start();
    }
    if !rest.trim().is_empty() {
        return Err(LaunchError::SurplusArguments {
            procedure: procedure.clone(),
            surplus: rest.trim().to_owned(),
        });
    }
    Ok(bound)
}

fn missing(procedure: &SnippetId, parameter: &IndexParameter) -> LaunchError {
    LaunchError::MissingArgument {
        procedure: procedure.clone(),
        name: parameter.name.clone(),
    }
}

/// The opening message: the command that loads the procedure with its
/// closure, a preamble mapping each bound name to its value and naming
/// the raw argument text as `$ARGUMENTS`, then the briefing.
pub fn compose(
    procedure: &SnippetId,
    bound: &[(String, String)],
    raw: &str,
    brief: Option<&str>,
) -> String {
    let mut text =
        format!("Run `instrukt docs get {procedure} --closure` and execute that procedure.\n");
    if !bound.is_empty() || !raw.trim().is_empty() {
        text.push('\n');
        for (name, value) in bound {
            text.push_str(&format!("{name}: {value}\n"));
        }
        text.push_str(&format!("$ARGUMENTS: {}\n", raw.trim()));
    }
    if let Some(brief) = brief.filter(|b| !b.trim().is_empty()) {
        text.push('\n');
        text.push_str(brief.trim_end());
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(name: &str, required: bool, rest: bool) -> IndexParameter {
        IndexParameter {
            name: name.into(),
            required,
            rest,
        }
    }

    fn id() -> SnippetId {
        SnippetId::parse("sdlc/procedure/build").unwrap()
    }

    fn pairs(bound: &[(String, String)]) -> Vec<(&str, &str)> {
        bound
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect()
    }

    #[test]
    fn tokens_bind_in_order_and_the_rest_entry_takes_the_remainder_verbatim() {
        let params = [param("slug", true, false), param("note", false, true)];
        let bound = bind_arguments(&id(), &params, "  demo  keep  this   spacing ").unwrap();
        assert_eq!(
            pairs(&bound),
            [("slug", "demo"), ("note", "keep  this   spacing")]
        );
        let only = bind_arguments(&id(), &params, "demo").unwrap();
        assert_eq!(pairs(&only), [("slug", "demo")]);
    }

    #[test]
    fn missing_required_and_surplus_arguments_refuse() {
        let params = [param("slug", true, false), param("phase", false, false)];
        assert!(matches!(
            bind_arguments(&id(), &params, " "),
            Err(LaunchError::MissingArgument { .. })
        ));
        assert!(matches!(
            bind_arguments(&id(), &params, "a b c"),
            Err(LaunchError::SurplusArguments { .. })
        ));
        assert!(matches!(
            bind_arguments(&id(), &[], "x"),
            Err(LaunchError::SurplusArguments { .. })
        ));
        assert_eq!(bind_arguments(&id(), &params, "a").unwrap().len(), 1);
    }
}
