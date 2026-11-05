# Deferral disposition — {slug}

Process the current `deferrals` artifact before any finalizer dispatch or handoff. Record a disposition per item in `log`, file admissible new scope with `instrukt todo create <new_slug> '<content>' --kind <kind> --approved` after satisfying the creation-admission questions, and record `instrukt todo stamp {slug} --deferrals-processed`. Artifact and stamp are record writes, not repository commit work.

Before starting, read the role binding with `instrukt todo show {slug}` and observe its recorded session through `instrukt sessions alive <id>`. Continue an available session with `instrukt sessions send <id> '<brief>'`; `run` is the start branch, not an implicit reuse operation. The runner records the returned runtime-native id per todo and role; preserve that binding. Choose the first available entry in the procedure's dispatch order or the global order. A reply or ended turn does not establish completion.
