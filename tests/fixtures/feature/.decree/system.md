# All machines

Every machine, the `emits` and `invokes` edges between them, and cron entry points.

- [deploy](machines/deploy/graph.md): Build, ask a person to approve, then ship.
- [develop](machines/develop/graph.md): Make a small code change with an AI agent, then run the tests.
- [feature](machines/feature/graph.md): Implement one feature spec with an AI agent, verify it, and commit.
- [router](machines/router/graph.md): Ask Claude to pick one of the options in the request.
- [ship](machines/ship/graph.md): Implement a feature, then deploy it.
- [triage](machines/triage/graph.md): Read a free-form request and hand it to the machine that should do the work.

```mermaid
flowchart LR
    deploy["deploy"]
    develop["develop"]
    feature["feature"]
    router["router"]
    ship["ship"]
    triage["triage"]
    feature -->|emits| feature
    triage -->|emits| develop
    triage -->|emits| feature
    feature -->|invokes| router
    ship -->|invokes| deploy
    ship -->|invokes| feature
    triage -->|invokes| router
```
