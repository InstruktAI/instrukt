# Preparation handoff — {slug}

The current preparation head is approved. Release your execution claim now with `instrukt todo update {slug} --unclaim`, while this preparation worktree still exists, and confirm through `instrukt todo show {slug}` that `claim.held` is false. That release records the integration occurrence. Finish this preparation assignment through the configured native operation. A refused release remains owed. The integrator then integrates and publishes the reviewed head before removing the preparation worktree and branches. Build stays pending until integration; later work starts in `<slug>/work` from updated main.

<!-- @config: runtime.finish_thread -->
