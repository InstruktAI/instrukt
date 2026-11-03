# Delivered workspace cleanup — {slug}

Delivery is recorded. Owe `worktree.remove` until `worktree.exists` is absent. Run from the project checkout, outside the removed worktree. Preserve a dirty or in-use worktree and report a failed removal; never force it. Inspect delivery and filesystem evidence. Delivery follows the work lane’s confirmed claim release and child-session cleanup; the integrator performs only workspace removal here, without claiming or mutating the delivered record.
