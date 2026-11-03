# Task runner route — {slug}

A task runs under runner eligibility, authorization, dependency and due-time rules, through `sdlc/procedure/lifecycle/task/run-task`. This work flow dispatches no task or repository builder. The run procedure records `--task-firing-started` and `--task-outcome success|error`, with `--reason` for a failure, on the particular occurrence.
