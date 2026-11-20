# All machines

Every machine, the `emits` and `invokes` edges between them, and cron entry points.

- [deploy](machines/deploy/graph.md): Build, ask a person to approve, then ship.
- [develop](machines/develop/graph.md): Implement a message, then test it.
- [hello](machines/hello/graph.md): Run one script.

```mermaid
flowchart LR
    deploy["deploy"]
    develop["develop"]
    hello["hello"]
    cron__develop_nightly_audit[/"cron: develop/nightly-audit"/]
    cron__develop_nightly_audit -->|cron| develop
```
