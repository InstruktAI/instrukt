# Delivered workspace cleanup — {slug}

Delivery is recorded. From the project checkout, enumerate every path and role branch in `attempts.retained`. Execute `worktree.remove` for the selected still-existing path, refresh and re-derive until every retained path is absent. Then execute `role-branch.remove` for each selected remaining role branch until all are absent. Each effect's `done_when` observes its selected resource.

Preserve dirty or in-use worktrees, unmerged branches and failed removals and report the failed obligation. Delivery follows the work lane's confirmed claim release; the integrator performs resource removal here against delivery and filesystem evidence. Retained attempt bindings remain evidence.
