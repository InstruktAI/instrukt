//! A fake runtime: a shell script the test writes, mapped as a `runtimes:`
//! entry of the temporary global layer. It is the runtime boundary the
//! sessions facade talks to, so the facade under test stays real.
//!
//! The script keeps its sessions in a directory of the test: `start`
//! prints `s<n>` (and a worktree path when `INSTRUKT_WORKTREE` is set),
//! keeping the opening message; `send` keeps each text; `alive` prints
//! the session's status file (`running` after start, `gone` without one);
//! `conversation` prints the session's conversation; `link` and
//! `link --remove` join and part two sessions; `search` names the sessions
//! whose conversation holds a text; `type` is the developer
//! typing into a session, which also reaches its linked sessions as the
//! developer's words.
#![allow(
    clippy::unwrap_used,
    dead_code,
    reason = "test setup: a failed step fails the test; each test crate uses what it needs"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use crate::common::TestEnv;

const SCRIPT: &str = r#"#!/bin/sh
set -eu
state="$1"
cmd="$2"
shift 2
mkdir -p "$state"
case "$cmd" in
  start)
    n=$(( $(cat "$state/counter" 2>/dev/null || echo 0) + 1 ))
    echo "$n" > "$state/counter"
    id="s$n"
    printf '%s' "$1" > "$state/$id.opening"
    echo running > "$state/$id.status"
    echo "start $id ${INSTRUKT_AGENT:-}" >> "$state/calls"
    if [ -f "$state/silent-start" ]; then
      # The session exists, but the runtime prints no id for it.
      exit 0
    fi
    if [ -f "$state/die-after-start" ]; then
      # The runner is killed after the session exists, before it learns
      # the id: kill the instrukt process that started this command.
      rm "$state/die-after-start"
      pid=$PPID
      for _ in 1 2 3 4 5; do
        case "$(ps -o comm= -p "$pid")" in
          *instrukt) kill -KILL "$pid"; exit 0 ;;
        esac
        pid=$(ps -o ppid= -p "$pid" | tr -d ' ')
      done
      echo "no instrukt ancestor" >&2
      exit 3
    fi
    echo "$id"
    if [ -n "${INSTRUKT_WORKTREE:-}" ]; then echo "$state/worktrees/$INSTRUKT_WORKTREE"; fi
    ;;
  send)
    id="$1"
    n=$(ls "$state" | grep -c "^$id\.sent\." || true)
    printf '%s' "$2" > "$state/$id.sent.$((n + 1))"
    echo "send $id" >> "$state/calls"
    ;;
  alive)
    if [ -f "$state/$1.status" ]; then cat "$state/$1.status"; else echo gone; fi
    ;;
  conversation)
    cat "$state/$1.conversation" 2>/dev/null || true
    ;;
  link)
    a="$1"
    if [ "$2" = "--remove" ]; then
      b="$3"
      grep -vx "$b" "$state/$a.links" > "$state/$a.links.new" || true
      mv "$state/$a.links.new" "$state/$a.links"
      echo "unlink $a $b" >> "$state/calls"
    else
      b="$2"
      echo "$b" >> "$state/$a.links"
      echo "$a" >> "$state/$b.links"
      echo "link $a $b" >> "$state/calls"
    fi
    ;;
  notify)
    printf '%s\n' "$*" >> "$state/notifications"
    ;;
  search)
    for file in "$state"/*.conversation; do
      [ -f "$file" ] || continue
      if grep -q -- "$1" "$file"; then basename "$file" .conversation; fi
    done
    ;;
  type)
    id="$1"
    printf 'developer: %s\n' "$2" >> "$state/$id.conversation"
    if [ -f "$state/$id.links" ]; then
      while read -r other; do
        printf 'developer (typed in %s): %s\n' "$id" "$2" >> "$state/$other.conversation"
      done < "$state/$id.links"
    fi
    ;;
  *)
    echo "unknown command $cmd" >&2
    exit 2
    ;;
esac
"#;

/// The fake runtime of one test.
pub struct FakeRuntime {
    pub name: String,
    script: PathBuf,
    state: PathBuf,
    link: bool,
    agent: Option<String>,
    history: bool,
}

impl FakeRuntime {
    /// A runtime named `name`, with the `link` capability when `link`.
    pub fn new(env: &TestEnv, name: &str, link: bool) -> Self {
        let dir = env.path(&format!("runtime-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("rt");
        std::fs::write(&script, SCRIPT).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let state = dir.join("sessions");
        std::fs::create_dir_all(&state).unwrap();
        Self {
            name: name.to_owned(),
            script,
            state,
            link,
            agent: None,
            history: false,
        }
    }

    /// The `runtimes:` entry mapping this runtime, as YAML list item.
    pub fn entry(&self) -> String {
        let base = format!("'{}' '{}'", self.script.display(), self.state.display());
        let env = self.name.to_uppercase();
        let mut entry = format!(
            "  - name: {name}\n    session_id_env: {env}_SESSION\n    start: \"{base} start\"\n    send: \"{base} send \\\"{{runtime.session_id}}\\\"\"\n    alive: \"{base} alive '{{runtime.session_id}}'\"\n    conversation: \"{base} conversation {{runtime.session_id}}\"\n",
            name = self.name,
        );
        entry.push_str(&format!("    notify: \"{base} notify\"\n"));
        if let Some(agent) = &self.agent {
            entry.push_str(&format!("    agent: {agent}\n"));
        }
        if self.history {
            entry.push_str(&format!(
                "    history:\n      search: \"{base} search\"\n      show: \"{base} conversation\"\n"
            ));
        }
        if self.link {
            entry.push_str(&format!(
                "    link: \"{base} link {{runtime.session_id}}\"\n"
            ));
        }
        entry
    }

    /// A session the developer already has open in this runtime.
    pub fn open_session(&self, id: &str) {
        std::fs::write(self.state.join(format!("{id}.status")), "needs-input\n").unwrap();
    }

    /// The developer types `text` into session `id`, in the runtime.
    pub fn developer_types(&self, id: &str, text: &str) {
        let status = std::process::Command::new(&self.script)
            .arg(&self.state)
            .arg("type")
            .arg(id)
            .arg(text)
            .status()
            .unwrap();
        assert!(status.success());
    }

    /// This runtime searches and shows past conversations (`history`).
    pub fn with_history(mut self) -> Self {
        self.history = true;
        self
    }

    /// This runtime is the agent CLI that starts `agent`.
    pub fn starting_agent(mut self, agent: &str) -> Self {
        self.agent = Some(agent.to_owned());
        self
    }

    /// Every start creates its session but prints no id for it.
    pub fn start_without_printing_ids(&self) {
        std::fs::write(self.state.join("silent-start"), "").unwrap();
    }

    /// The next start creates its session, then kills the runner that
    /// asked for it before the runner hears the id.
    pub fn kill_runner_on_next_start(&self) {
        std::fs::write(self.state.join("die-after-start"), "").unwrap();
    }

    /// Sets the status `alive` reports for session `id`.
    pub fn set_status(&self, id: &str, status: &str) {
        std::fs::write(
            self.state.join(format!("{id}.status")),
            format!("{status}\n"),
        )
        .unwrap();
    }

    /// The notifications the runtime delivered, one argument list per
    /// line.
    pub fn notifications(&self) -> Vec<String> {
        std::fs::read_to_string(self.state.join("notifications"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The calls the runtime received, one per line.
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.state.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The opening message session `id` started with.
    pub fn opening(&self, id: &str) -> String {
        std::fs::read_to_string(self.state.join(format!("{id}.opening"))).unwrap()
    }

    /// How many sessions this runtime started.
    pub fn started_count(&self) -> usize {
        std::fs::read_to_string(self.state.join("counter"))
            .map(|n| n.trim().parse().unwrap())
            .unwrap_or(0)
    }

    /// How many texts session `id` was sent.
    pub fn sent_count(&self, id: &str) -> usize {
        (1..)
            .take_while(|n| self.state.join(format!("{id}.sent.{n}")).exists())
            .count()
    }

    /// The `n`th text (from 1) sent to session `id`.
    pub fn sent(&self, id: &str, n: usize) -> String {
        std::fs::read_to_string(self.state.join(format!("{id}.sent.{n}"))).unwrap()
    }

    pub fn state_dir(&self) -> &Path {
        &self.state
    }
}

/// Writes the global layer: `modules` switches, then the runtimes;
/// anomaly notification is left at its default.
pub fn global_layer(env: &TestEnv, modules: &str, runtimes: &[&FakeRuntime]) {
    global_layer_with(env, modules, runtimes, "");
}

/// As [`global_layer`], with `extra` YAML appended (an agent order).
pub fn global_layer_with(env: &TestEnv, modules: &str, runtimes: &[&FakeRuntime], extra: &str) {
    let mut text = format!("schema_version: 1\nmodules: {{{modules}}}\nruntimes:\n");
    for runtime in runtimes {
        text.push_str(&runtime.entry());
    }
    text.push_str(extra);
    let path = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}
