//! A fake T3 Code host command: a `t3` script on the test's `PATH`, ahead
//! of any real one, answering `t3 thread start|send|show` the way the T3
//! server on the developer's computer does. instrukt's shipped `t3`
//! runtime entry drives it unchanged, so the entry's templates are what is
//! under test; a real `t3` is never called.
//!
//! Threads live in a directory of the test: `start` creates `thread:<n>`
//! in the folder it runs in (or a worktree of the requested branch),
//! `send` appends to a thread, and `show` prints a thread's state, as T3's
//! JSON with `--json`, or its latest messages. An unknown thread is "no
//! longer available", as T3 says.
#![allow(
    clippy::unwrap_used,
    dead_code,
    reason = "test setup: a failed step fails the test; each test crate uses what it needs"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use crate::common::TestEnv;

const SCRIPT: &str = r#"#!/bin/sh
set -eu
state="__STATE__"
mkdir -p "$state/threads"
echo "$*" >> "$state/calls"
if [ -f "$state/down" ]; then
  echo "connect ECONNREFUSED 127.0.0.1:3773" >&2
  exit 1
fi
[ "$1" = thread ] || { echo "unknown command $1" >&2; exit 2; }
sub="$2"
shift 2
json=0
worktree=""
branch=""
while [ $# -gt 0 ]; do
  case "$1" in
    --json) json=1; shift ;;
    --messages) shift 2 ;;
    --worktree) worktree="$2"; shift 2 ;;
    --branch) branch="$2"; shift 2 ;;
    *) break ;;
  esac
done
gone() { echo "Thread $1 is no longer available." >&2; exit 1; }
case "$sub" in
  start)
    n=$(( $(cat "$state/counter" 2>/dev/null || echo 0) + 1 ))
    echo "$n" > "$state/counter"
    id="thread:$n"
    dir="$state/threads/$n"
    mkdir -p "$dir"
    pwd -P > "$dir/folder"
    echo running > "$dir/status"
    if [ -n "$worktree" ]; then
      echo "$state/worktrees/$branch" > "$dir/worktree"
      echo "$worktree" > "$dir/base"
    fi
    printf 'user: %s\n' "$2" > "$dir/messages"
    echo 1 > "$dir/count"
    echo "$id"
    ;;
  send)
    dir="$state/threads/${1#thread:}"
    [ -d "$dir" ] || gone "$1"
    printf 'user: %s\n' "$2" >> "$dir/messages"
    echo $(( $(cat "$dir/count") + 1 )) > "$dir/count"
    ;;
  show)
    dir="$state/threads/${1#thread:}"
    [ -d "$dir" ] || gone "$1"
    if [ "$json" = 1 ]; then
      path=$(cat "$dir/worktree" 2>/dev/null || true)
      pending=$(cat "$dir/pending" 2>/dev/null || echo 0)
      archived=$(cat "$dir/archived" 2>/dev/null || echo false)
      printf '{"thread":{"threadId":"%s","pullRequests":[],"status":"%s","worktreePath":"%s","pendingRequestCount":%s,"archived":%s,"settled":false},"messages":[]}\n' \
        "$1" "$(cat "$dir/status")" "$path" "$pending" "$archived"
    else
      cat "$dir/messages"
    fi
    ;;
  *)
    echo "unknown thread subcommand $sub" >&2
    exit 2
    ;;
esac
"#;

/// The fake `t3` of one test.
pub struct FakeT3 {
    script: PathBuf,
    state: PathBuf,
}

impl FakeT3 {
    /// Installs `t3` in a directory of the test and puts it first on the
    /// test's `PATH`.
    pub fn install(env: &mut TestEnv) -> Self {
        let bin = env.path("t3-bin");
        let state = env.path("t3-state");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let script = bin.join("t3");
        std::fs::write(
            &script,
            SCRIPT.replace("__STATE__", &state.display().to_string()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::var("PATH").unwrap_or_default();
        env.set_env("PATH", &format!("{}:{path}", bin.display()));
        Self { script, state }
    }

    /// A thread the developer started in T3 themselves; its id.
    pub fn developer_starts_thread(&self) -> String {
        let out = std::process::Command::new(&self.script)
            .args(["thread", "start", ".", "Look into the flaky test."])
            .current_dir(&self.state)
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    /// Every `t3` command line it received, one per line.
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.state.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The ids of the threads started, in order.
    pub fn threads(&self) -> Vec<String> {
        let count: usize = std::fs::read_to_string(self.state.join("counter"))
            .map(|n| n.trim().parse().unwrap())
            .unwrap_or(0);
        (1..=count).map(|n| format!("thread:{n}")).collect()
    }

    /// The folder thread `id` was started in.
    pub fn folder(&self, id: &str) -> PathBuf {
        PathBuf::from(self.read(id, "folder").trim())
    }

    /// The messages thread `id` holds, one per line.
    pub fn messages(&self, id: &str) -> String {
        self.read(id, "messages")
    }

    /// How many messages thread `id` received.
    pub fn message_count(&self, id: &str) -> usize {
        self.read(id, "count").trim().parse().unwrap()
    }

    /// Sets the T3 status of thread `id`.
    pub fn set_status(&self, id: &str, status: &str) {
        self.write(id, "status", status);
    }

    /// Thread `id` waits on `count` pending requests.
    pub fn set_pending(&self, id: &str, count: u32) {
        self.write(id, "pending", &count.to_string());
    }

    /// The base thread `id`'s worktree was created from.
    pub fn worktree_base(&self, id: &str) -> String {
        self.read(id, "base").trim().to_owned()
    }

    /// The worktree path thread `id` runs in.
    pub fn worktree(&self, id: &str) -> PathBuf {
        PathBuf::from(self.read(id, "worktree").trim())
    }

    /// The T3 server stops answering.
    pub fn server_down(&self) {
        std::fs::write(self.state.join("down"), "").unwrap();
    }

    /// Archives thread `id`.
    pub fn archive(&self, id: &str) {
        self.write(id, "archived", "true");
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.state
            .join("threads")
            .join(id.strip_prefix("thread:").unwrap())
    }

    fn read(&self, id: &str, file: &str) -> String {
        std::fs::read_to_string(self.dir(id).join(file)).unwrap()
    }

    fn write(&self, id: &str, file: &str, text: &str) {
        std::fs::write(self.dir(id).join(file), format!("{text}\n")).unwrap();
    }
}
