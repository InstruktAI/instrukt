//! `instrukt repo commit`: an attributed commit — the acting agent as
//! author, the session it ran in and the caller's declaration of human
//! involvement as trailers, and the work item when given. What the
//! environment does not resolve is left out, never guessed.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::{ConfigError, Configuration, Env};
use crate::git;
use crate::home::Home;
use crate::ids::{CommitSha, TodoSlug};
use crate::interfaces::CoreWiring;

use super::_error::RepositoryError;
use super::_git;

/// The trailer keys instrukt owns, each with the door that supplies its
/// fact. A caller's message holding any of them is refused.
pub const OWNED_TRAILERS: [(&str, &str); 4] = [
    (
        "Session",
        "the active runtime's session variable (instrukt writes it when one is set)",
    ),
    ("Human-Involved", "`--human-involved true|false`"),
    ("Todo", "`--todo <slug>`"),
    ("Planned-Todo", "`--todo <slug>`"),
];

/// What the caller asks to commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest {
    /// The caller's message: a subject and an explanatory body.
    pub message: String,
    /// The caller's declaration of whether a human was involved.
    pub human_involved: bool,
    /// The work item the commit belongs to.
    pub todo: Option<TodoSlug>,
    /// Commit exactly these paths as they are in the working tree, leaving
    /// the rest of the index as it is; empty commits what is staged.
    pub only: Vec<PathBuf>,
}

/// The association a commit names with a work item.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TodoTrailer {
    Recorded(TodoSlug),
    Planned(TodoSlug),
}

/// Makes an attributed commit in the repository containing `cwd`, running
/// the project's own hooks as any commit does; returns its id. A message
/// carrying an owned trailer is refused before anything is committed.
pub fn commit(
    home: &Home,
    cwd: &Path,
    env: &Env,
    wiring: &CoreWiring,
    request: &CommitRequest,
) -> Result<CommitSha, RepositoryError> {
    let started = Instant::now();
    refuse_owned_trailers(&request.message)?;
    let root = _git::root(cwd)?;
    let configuration = Configuration::load(home, Some(&root))?;
    let author = agent_author(&configuration, env)?;
    let session = session(&configuration, env)?;
    let todo = match &request.todo {
        Some(slug) => Some(todo_trailer(&configuration, wiring, slug)?),
        None => None,
    };
    let message = compose(
        &request.message,
        session.as_deref(),
        request.human_involved,
        todo.as_ref(),
    );
    let mut args: Vec<String> = vec![
        "commit".into(),
        "--quiet".into(),
        "--cleanup=whitespace".into(),
        "--message".into(),
        message,
    ];
    if let Some(author) = &author {
        args.push(format!("--author={author}"));
    }
    if request.only.is_empty() {
        git::run(cwd, &args)?;
    } else {
        commit_only(cwd, &mut args, &request.only)?;
    }
    let sha = CommitSha::parse(&git::run(cwd, ["rev-parse", "HEAD"])?)?;
    tracing::debug!(
        event = "commit_created",
        sha = %sha,
        agent_author = author.is_some(),
        session = session.is_some(),
        human_involved = request.human_involved,
        only_paths = request.only.len(),
        duration_ms = started.elapsed().as_millis() as u64,
    );
    Ok(sha)
}

/// Commits exactly `paths`: untracked ones are made known to the index
/// (intent to add) so `--only` takes them, and made unknown again when the
/// commit does not happen.
fn commit_only(
    cwd: &Path,
    args: &mut Vec<String>,
    paths: &[PathBuf],
) -> Result<(), RepositoryError> {
    let mut listing = vec![
        "ls-files".to_owned(),
        "-z".to_owned(),
        "--others".to_owned(),
        "--exclude-standard".to_owned(),
        "--".to_owned(),
    ];
    listing.extend(paths.iter().map(|path| path.display().to_string()));
    let untracked: Vec<String> = git::run(cwd, &listing)?
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    if !untracked.is_empty() {
        let mut add = vec![
            "add".to_owned(),
            "--intent-to-add".to_owned(),
            "--".to_owned(),
        ];
        add.extend(untracked.iter().cloned());
        git::run(cwd, &add)?;
    }
    args.push("--only".into());
    args.push("--".into());
    args.extend(paths.iter().map(|path| path.display().to_string()));
    let committed = git::run(cwd, &*args);
    if committed.is_err() && !untracked.is_empty() {
        let mut forget = vec![
            "rm".to_owned(),
            "--cached".to_owned(),
            "--quiet".to_owned(),
            "--ignore-unmatch".to_owned(),
            "--".to_owned(),
        ];
        forget.extend(untracked);
        if let Err(err) = git::run(cwd, &forget) {
            tracing::warn!(event = "commit_index_not_restored", error = %err);
        }
    }
    committed?;
    Ok(())
}

/// Refuses a message with a line whose key is a trailer instrukt owns, in
/// any letter case.
fn refuse_owned_trailers(message: &str) -> Result<(), RepositoryError> {
    for (index, line) in message.lines().enumerate() {
        let Some((key, _)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if let Some((owned, door)) = OWNED_TRAILERS
            .iter()
            .find(|(owned, _)| owned.eq_ignore_ascii_case(key))
        {
            return Err(RepositoryError::OwnedTrailer {
                line: index + 1,
                text: line.to_owned(),
                key: owned,
                door,
            });
        }
    }
    Ok(())
}

/// The active agent's configured git author; `None` outside an agent
/// session. An agent without one, or with one git would not take as given,
/// is refused.
fn agent_author(
    configuration: &Configuration,
    env: &Env,
) -> Result<Option<String>, RepositoryError> {
    let Some(agent) = configuration
        .active_agent(env)
        .and_then(|entry| entry.agent.clone())
    else {
        return Ok(None);
    };
    if configuration.agent_git_author(&agent)?.is_none() {
        return Err(RepositoryError::AgentAuthorMissing { agent });
    }
    // Interpolation is configuration's: the author is read resolved in the
    // invocation's environment, never as the raw stored text.
    let author = configuration.get_str(&format!("agents.{agent}.git_author"), env)?;
    if !is_name_and_email(&author) {
        return Err(RepositoryError::AgentAuthorMalformed {
            agent,
            value: author,
        });
    }
    Ok(Some(author))
}

/// `Name <email>`: anything else makes git search earlier commits for a
/// matching author, which would be a guess.
fn is_name_and_email(author: &str) -> bool {
    let Some((name, rest)) = author.split_once('<') else {
        return false;
    };
    let Some(email) = rest.strip_suffix('>') else {
        return false;
    };
    !name.trim().is_empty() && !email.trim().is_empty() && !email.contains(['<', '>'])
}

/// `<runtime>:<id>` of the active runtime; `None` when no runtime's session
/// variable is set.
fn session(configuration: &Configuration, env: &Env) -> Result<Option<String>, RepositoryError> {
    match configuration.active_runtime(env) {
        Ok(entry) => Ok(env
            .get(&entry.session_id_env)
            .map(|id| format!("{}:{id}", entry.name))),
        Err(ConfigError::NoActiveRuntime) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// The association to write for `slug`: as given while todos is off (or the
/// project has no slug to ask about); with todos on, recorded when the item
/// exists and planned otherwise.
fn todo_trailer(
    configuration: &Configuration,
    wiring: &CoreWiring,
    slug: &TodoSlug,
) -> Result<TodoTrailer, RepositoryError> {
    let (Some(existence), Some(project)) = (&wiring.todo_existence, configuration.project()?)
    else {
        return Ok(TodoTrailer::Recorded(slug.clone()));
    };
    Ok(if existence.todo_exists(&project, slug)? {
        TodoTrailer::Recorded(slug.clone())
    } else {
        TodoTrailer::Planned(slug.clone())
    })
}

/// The caller's message followed, after a blank line, by instrukt's
/// trailers in their fixed order.
fn compose(
    message: &str,
    session: Option<&str>,
    human_involved: bool,
    todo: Option<&TodoTrailer>,
) -> String {
    let mut out = message.trim_end().to_owned();
    out.push_str("\n\n");
    if let Some(session) = session {
        out.push_str(&format!("{}: {session}\n", OWNED_TRAILERS[0].0));
    }
    out.push_str(&format!("{}: {human_involved}\n", OWNED_TRAILERS[1].0));
    match todo {
        Some(TodoTrailer::Recorded(slug)) => {
            out.push_str(&format!("{}: {slug}\n", OWNED_TRAILERS[2].0))
        }
        Some(TodoTrailer::Planned(slug)) => {
            out.push_str(&format!("{}: {slug}\n", OWNED_TRAILERS[3].0))
        }
        None => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owned_key_is_refused_in_any_letter_case_and_others_pass() {
        assert!(refuse_owned_trailers("subject\n\nbody\nSigned-off-by: a <a@b>").is_ok());
        for (key, _) in OWNED_TRAILERS {
            let message = format!("subject\n\n{}: x", key.to_lowercase());
            match refuse_owned_trailers(&message) {
                Err(RepositoryError::OwnedTrailer {
                    line, key: refused, ..
                }) => {
                    assert_eq!(line, 3);
                    assert_eq!(refused, key);
                }
                other => panic!("{key}: {other:?}"),
            }
        }
    }

    #[test]
    fn only_name_and_email_is_taken_as_an_author() {
        assert!(is_name_and_email("Agent Smith <agent@example.test>"));
        for bad in ["agent", "<a@b>", "Agent <>", "Agent <a@b", "A <a<b>"] {
            assert!(!is_name_and_email(bad), "{bad}");
        }
    }

    #[test]
    fn trailers_follow_a_blank_line_in_their_fixed_order() {
        let slug = TodoSlug::parse("t").unwrap();
        let composed = compose(
            "subject\n\nbody\n\n",
            Some("host:1"),
            true,
            Some(&TodoTrailer::Planned(slug)),
        );
        let lines: Vec<&str> = composed.lines().collect();
        let keys: Vec<&str> = lines[4..]
            .iter()
            .map(|line| line.split_once(':').map_or("", |(key, _)| key))
            .collect();
        assert!(lines[3].is_empty());
        assert_eq!(
            keys,
            [
                OWNED_TRAILERS[0].0,
                OWNED_TRAILERS[1].0,
                OWNED_TRAILERS[3].0
            ]
        );
        let bare = compose("subject", None, false, None);
        assert_eq!(bare.lines().count(), 3);
    }
}
