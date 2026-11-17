---
description: The direct runtime — instrukt starts the agent program itself for each turn of a company's conversation, resuming that conversation's session, with only the tools the role's grants give, and posts what the run returns.
---
# Direct Runtime — Spec

## What it is

The direct runtime is the runtime in which instrukt starts the agent program
itself. It sits beside T3 and is chosen per computer with `runner.runtime:
direct` (`project/instrukt/spec/feature/runner/session-continuity`). It carries
a company's conversations: each addressed message is one run of the agent
program that resumes the conversation's session, and the run ends when the turn
ends. Between runs nothing of that agent is running. Each run starts with the
agent program's own tool permissions set so that it can run only the instrukt
commands its role's grants give, and nothing else. A company's container runs
this runtime.

T3 stays the runtime for a developer's own work: it runs agent programs for a
person at their computer and offers no way to limit what a session can reach.

## Canonical fields

### What the runtime carries

| Feature (`project/instrukt/spec/feature/runner/session-continuity`) | On the direct runtime |
| --- | --- |
| Conversations (a company home's declared connector) | Carried: keyed `start`, `send` as a run, `read_state`, and the turn's output as the run's result. |
| Standing agents, todo pickup, recurring tasks | Not carried. Enabling them is refused naming the runtime and the missing capability. |
| History extraction | Not carried: the runtime has no conversation source. History reports extraction unavailable. |
| Conversation links | Not carried. `instrukt discuss` is refused naming the runtime. |
| Default anomaly notification | Not carried; an explicit `runner.notify` command works as on any runtime. |
| Operational sessions surface (`instrukt sessions list|resolve-start`) | Not carried. It is refused naming the runtime. |

A person cannot join a session in an interface, follow a run while it works or
type into it: the session exists only as the agent program's own record of it,
which instrukt resumes and does not read.

### The agent program

A profile in `agents.yaml` keeps deciding which program and model run
(`project/instrukt/spec/feature/configuration/agent-selection`). On this
runtime the profile's `kind` names the agent program and its `instance` names
the account: instrukt runs the program with a configuration directory of that
instance's own, `~/.instrukt/runtimes/direct/<instance>/`, readable by its owner
only. The program keeps its login and its session transcripts there; nothing of
the developer's own configuration of that program is read.

| Kind | On the direct runtime |
| --- | --- |
| `claude` (Claude Code) | Supported, as specified below. |
| `codex` (Codex CLI) | Not ready: its documentation offers no way to let a run execute only the instrukt commands a role is given while keeping it from reading files and editing them. Selection skips such a profile with that reason. |
| Any other kind | Not ready, with the same reason until its documentation shows otherwise. |

A profile is ready when its program is installed and its account is signed in
within the instance's configuration directory. The operator signs in once by
running the program with that directory; `instrukt agents status` names a
profile that is not signed in and that door.

### One turn, one run

A conversation's session is identified by a UUID instrukt makes when it starts
the session, recorded as the session `direct:<uuid>` in the conversation's
binding before the first run. Each addressed message is delivered as one run:

1. The runner records the message as attempted, then starts the program in
   the program's non-interactive mode with the framed input as its prompt
   (`project/instrukt/spec/feature/runner/conversations`). The first run of a
   session names the session's UUID; every later run resumes it.
2. The runner-authored assignment envelope of the conversation is appended to
   the program's system prompt on every run, with every other run option,
   because the program does not keep them between runs.
3. The run's environment carries `INSTRUKT_SESSION=direct:<uuid>`, which is
   the caller identity this runtime recognises for `instrukt` commands run
   inside it.
4. The run ends when its turn ends. Its result, as the program returns it in
   its structured output, is the turn's output: instrukt reads it from the run
   and from nowhere else, and does not read the program's own store. It is
   stored as the message's answer in the same transaction that marks the
   message answered, and is then posted as the conversation contract states.

| A run | Outcome |
| --- | --- |
| Returns a result | The result is the answer; a bare `NOOP` posts nothing. |
| Ends with an error, or without a result | The message is `failed` with its cause, and the person gets the connector's fixed reply that the agent could not answer. |
| Runs longer than 30 minutes | It is stopped, with every process it started; then as for an error. |
| Is interrupted, because the runner stopped or the computer restarted | After the restart the message is `failed` as interrupted, and the person gets the same fixed reply. |

A run that started is never run again: what its tools did cannot be known
without it, and a second run could repeat it. A message whose run did not start
stays pending and is delivered when the runner can start it. One run of a
conversation runs at a time, and later messages wait as the conversation
contract states. Runs of different conversations run side by side.

What survives a restart is what the state database keeps (the binding with its
session, each message with its state and each answer) and the program's own
record of the session in the instance's configuration directory, which the
next run resumes.

### What a run can reach

A role's grants decide the run's tools
(`project/instrukt/spec/feature/configuration/roles-and-capabilities`). This
runtime offers an agent exactly two instrukt commands:

| Command | Action it needs |
| --- | --- |
| `instrukt mcp …` | `mcp-use` |
| `instrukt docs get …` | `docs-read` |

A run may run those whose action the role's effective grants contain, and
nothing else: no reading or writing of files, no other command, no web fetch or
search, no tool server of the program's own. A role whose grants contain
neither has no tools at all. A call outside these is refused by the program at
once; no run waits for an approval.

For Claude Code each run is started so:

| What | Setting |
| --- | --- |
| Non-interactive run with a structured result | `-p` with `--output-format json`; the result is the output's `result`, an error is `is_error` or a non-zero exit. |
| Session | `--session-id <uuid>` on the first run, `--resume <uuid>` on every later one. |
| Model | `--model` and `--effort` from the selected profile's tier chain. |
| Configuration | `CLAUDE_CONFIG_DIR` set to the instance's directory; `--setting-sources user`; a settings file instrukt writes for the run, passed with `--settings`. |
| Instruction files | `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`: no `CLAUDE.md` or memory file is loaded. |
| Working directory | An empty directory of the conversation's own under `~/.instrukt/runtimes/direct/work/`, never the company home. |
| Built-in tools | `--tools Bash` when the role is given a command, `--tools ""` when it is given none. |
| Tool servers | `--strict-mcp-config` with no server, and the program's connectors switched off (`ENABLE_CLAUDEAI_MCP_SERVERS=false`). |
| Permissions | Mode `dontAsk`, `--permission-prompts none`, and `permissions.denyInsteadOfPrompting`; `permissions.allow` holds `Bash(instrukt mcp *)` and `Bash(instrukt docs get *)` as the grants give them; `permissions.blockReadsOutsideWorkingDirectories` is on. |
| Sandbox | `sandbox.enabled` with `failIfUnavailable`, `allowUnsandboxedCommands: false` and `autoAllowBashIfSandboxed: false`; no allowed network domain; reads of the user's home directory denied except the run's working directory; the granted instrukt commands listed in `excludedCommands`, so that they alone run outside it with their own access. |

The program's built-in set of read-only commands, which it runs without
approval in every mode, therefore sees only the empty working directory and
runs inside the sandbox without network.

### What this runtime refuses

- Enabling a feature it does not carry, as listed above.
- A profile of a kind that is not ready on it.
- `instrukt discuss`, `instrukt sessions list|resolve-start` and every other
  command whose runtime capability it lacks, naming the runtime.

## Use cases

### UC-DR1: A company's agent answers using only the tools its role gives

```gherkin
Given a developer runs a company's conversations on the direct runtime
And the conversation role is given the company's MCP servers and documentation
When a member asks the agent for work that needs one of those servers
Then the agent's call through instrukt reaches the server and the answer is posted
But when the agent tries to read a file, run another command or reach the network itself
Then the program refuses it at once and no run waits for an approval
```

### UC-DR2: Continue a conversation across turns and a restart

```gherkin
Given a member has talked with the agent on the direct runtime
When the runner restarts and the member writes again in that conversation
Then the agent continues the same session and answers with what was said before
But when a run is interrupted by the restart
Then that message is not run again and the member is told the agent could not answer
```

### UC-DR3: Run a role that has no tools at all

```gherkin
Given a developer gives a role no grants that this runtime turns into tools
When a member addresses an agent in that role
Then its run has no tools at all and its answer is posted
```

## Known caveats

- The agent runs as the same operating-system user as instrukt. Its tool
  permissions and the sandbox are what keep it from the company's secrets file
  and the stored logins. Claude Code documents that a command rule matches the
  command text it writes and is not a security boundary around a program
  invoked in another form; the empty working directory, the blocked reads and
  the sandbox are what stand behind the rules. A separation in which the agent
  runs as another operating-system user and `instrukt mcp` asks the runner is
  later work.
- One runtime is chosen per computer: a computer on the direct runtime carries
  no developer work through T3.
- A run that started and was interrupted is not run again; the person asks
  again.
- The answer is the run's final result. What the agent wrote before its final
  result in that turn is not posted.
- Codex is not supported on this runtime.

## See Also

- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md) — the runtime adapter and which runtime carries which feature.
- [project/instrukt/spec/feature/runner/conversations](conversations.md)
- [project/instrukt/spec/feature/runner/mcp-access](mcp-access.md)
- [project/instrukt/spec/feature/configuration/agent-selection](../configuration/agent-selection.md)
- [project/instrukt/spec/feature/configuration/roles-and-capabilities](../configuration/roles-and-capabilities.md)
- https://code.claude.com/docs/en/headless — non-interactive runs with `-p`, `--output-format json`, resuming with `--resume`, and `--permission-prompts none`: a call that would prompt is denied and the run continues.
- https://code.claude.com/docs/en/cli-reference — `--session-id` (a UUID the caller chooses), `--resume`, `--tools` (`""` disables every built-in tool), `--model`, `--effort`, `--settings`, `--setting-sources`, `--strict-mcp-config`, `--append-system-prompt`, `--permission-mode`.
- https://code.claude.com/docs/en/sessions — run options are not restored on resume; transcripts live under the configuration directory; two runs resuming one session at once can lose messages.
- https://code.claude.com/docs/en/permissions — rule syntax and order (deny, ask, allow), `dontAsk`, the built-in read-only commands that run in every mode, and that a command rule is not a security boundary.
- https://code.claude.com/docs/en/settings-reference — `permissions.blockReadsOutsideWorkingDirectories`, `permissions.denyInsteadOfPrompting`, and the sandbox settings `failIfUnavailable`, `allowUnsandboxedCommands`, `autoAllowBashIfSandboxed` and `excludedCommands`.
- https://code.claude.com/docs/en/sandboxing — the sandbox (Seatbelt on macOS, bubblewrap on Linux), its read and network limits, and that excluded commands run with full access.
- https://code.claude.com/docs/en/env-vars — `CLAUDE_CONFIG_DIR` and `CLAUDE_CODE_DISABLE_CLAUDE_MDS`.
- https://code.claude.com/docs/en/authentication — where the login is kept (the macOS Keychain keyed to the configuration directory, or `.credentials.json` in it).
- https://code.claude.com/docs/en/mcp — `ENABLE_CLAUDEAI_MCP_SERVERS` and the connectors it switches off.
- https://learn.chatgpt.com/docs/non-interactive-mode, https://learn.chatgpt.com/docs/agent-configuration/rules, https://learn.chatgpt.com/docs/agent-approvals-security — Codex runs without a terminal and resumes, but its command rules govern only what runs outside its sandbox, commands and file reads run inside it, and no switch removes its file-editing tool.
