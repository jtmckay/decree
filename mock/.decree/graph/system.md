# All machines

Every machine, the `emits` and `invokes` edges between them, and cron entry points.

- [deploy](deploy.md): Build, ask a person to approve, then ship.
- [develop](develop.md): Make a small code change with an AI agent, then run the tests.
- [feature](feature.md): Implement one feature spec with an AI agent, verify it, and commit.
- [hello](hello.md): Run one script.
- [local_router](local_router.md): Ask the local classifier to score the options in the request.
- [router](router.md): Ask Claude to pick one of the options in the request.
- [ship](ship.md): Implement a feature, then deploy it.
- [sort_document](sort_document.md): File one scanned document as an invoice, a receipt or other paperwork.
- [triage](triage.md): Read a free-form request and hand it to the machine that should do the work.

```mermaid
flowchart LR
    deploy["deploy"]
    develop["develop"]
    feature["feature"]
    hello["hello"]
    local_router["local_router"]
    router["router"]
    ship["ship"]
    sort_document["sort_document"]
    triage["triage"]
    cron__nightly_audit[/"cron: nightly-audit"/]
    cron__nightly_audit -->|cron| develop
    feature -->|emits| feature
    triage -->|emits| develop
    triage -->|emits| feature
    feature -->|invokes| router
    ship -->|invokes| deploy
    ship -->|invokes| feature
    sort_document -->|invokes| local_router
    sort_document -->|invokes| router
    triage -->|invokes| router
```
