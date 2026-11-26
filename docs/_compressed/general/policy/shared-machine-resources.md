---
source_sha256: bb803b9c40fd1a7b5765698fc5d8d9e189eb2111d5e6a3080ca8baf2f4f3b363
model: claude-opus-5-5
---
- MUST keep 20 GiB free on shared volume -- full disk stops everyone.
- MUST remove regenerable output (`target/`, `node_modules`, `.venv`, caches, temp homes) of checkouts you create when done, then unneeded checkout, keeping committed/unregenerable work; briefs name it; clean finished delegates' output -- creator owns output.
- NEVER shell-delete todo worktree output or worktree -- instrukt reclaims idle/delivered ones.
- MUST check space before build/install/full test in fresh checkout: <20 GiB clean own output, still short stop and report space, largest dirs; todo worktree: `instrukt repo reclaim`, before or during build.
- MUST cite existing evidence (hook run, CI, same commit) instead of rebuilding elsewhere -- duplicates fill disk.
- NEVER clean another live session's output; ended session's is its delegator's.
- MUST run temporary processes in tracked group with guaranteed teardown (`trap`/`finally`/group kill) once evidence captured -- orphans starve host.
