# ainl-procedure-learning

Reusable procedure-learning crate for AINL hosts.

This crate intentionally has no `openfang-*` dependencies. It turns portable
`ExperienceBundle` values into `ProcedureArtifact` candidates, scores reuse,
generates simple failure-aware patches, and renders host-neutral artifacts such
as Markdown skills or AINL compact skeletons.
