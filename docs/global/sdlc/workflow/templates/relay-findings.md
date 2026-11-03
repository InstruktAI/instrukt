# Review changes requested — {slug}

The current changes-requested verdict binds the current evidence. Send `review-findings` to the recorded live builder under `sdlc/procedure/lifecycle/fix-review-findings`. Resolve each finding or record an evidence-backed disposition, commit the fixes and stamp `instrukt todo stamp {slug} --build-complete`. Reuse is `send`, not an unconditional new start; expose an unavailable builder before recovery.

Before starting, read the role binding with `instrukt todo show {slug}` and observe its recorded session through `instrukt sessions alive <id>`. Continue an available session with `instrukt sessions send <id> '<brief>'`; `run` is the start branch, not an implicit reuse operation. The runner records the returned runtime-native id per todo and role; preserve that binding. Choose the first available entry in the procedure's dispatch order or the global order. A reply or ended turn does not establish completion.
