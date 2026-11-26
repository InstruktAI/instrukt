---
source_sha256: bb803b9c40fd1a7b5765698fc5d8d9e189eb2111d5e6a3080ca8baf2f4f3b363
model: claude-opus-5-5
---
- MUST keep 20 GiB free on the shared volume; check before building in a fresh checkout; below -> clean own output; still below -> stop and report.
- whoever creates a checkout removes its regenerable output (`target/`, `node_modules`, `.venv`, caches, temp homes) and the checkout when done; a delegator names cleanup in the brief and removes what finished delegates left.
- todo worktrees are instrukt's: NEVER delete their output or worktree by shell; reclaim with `instrukt repo reclaim`.
- PREFER reusing existing evidence over rebuilding; NEVER clean another live session's output.
- MUST spawn temporary processes in a tracked group torn down by guaranteed cleanup once evidence is captured.
