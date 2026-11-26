---
source_sha256: ec71b254ef9d3d56f604fe681e1b765e1842063828886f7668845ec24effbe6b
model: claude-opus-5-5
---
- memory = stale cache: MUST return to the source before concluding; confidence without evidence = most dangerous state.
- ground both axes: state truth inward (docs, code) and solution truth outward (reference implementations, current vendor docs, production architectures).
- NEVER present premature findings as truth (inhale sparring excepted); state assumptions as declarations ("assuming X because Y"), NEVER questions.
- MUST re-anchor after interruption or context switch.
- a pokeable local surface (flag, config, code path) -> scan prior art first; ground the substrate (where state belongs, what dependencies provide) before the mechanism; build only the gap.
- "impossible", "needs a fork", new own infrastructure -> only on vendor docs + one reference implementation + one production architecture.
