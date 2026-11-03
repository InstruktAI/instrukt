# ARCHITECTURE revise — {slug}

Load `sdlc/procedure/lifecycle/prepare/architecture` inline and carry its architecture responsibility.
Owed: revision answering current review findings of `architecture` against the checked `input` and current grounding.
Read `architecture-review-findings`, resolve each finding under the owning procedure, and write the revised artifact. This is a revision, with no new dispatch reason or synthetic digest-diff header.
Author the durable design in domain/design snippets and keep `architecture` as its thin index/log. Where architecture earns no separate pass, record the reason with `instrukt todo stamp {slug} --architecture-skipped '<reason>'`.
Write with `instrukt todo artifacts write {slug} architecture '<content>'`, then re-enter preparation.
