# All machines

Every machine, the `emits` edges between them, and cron entry points.

```mermaid
flowchart LR
    deploy["deploy"]
    develop["develop"]
    feature["feature"]
    hello["hello"]
    triage["triage"]
    cron__nightly_audit[/"cron: nightly-audit"/]
    cron__nightly_audit -->|cron| develop
    feature -->|emits| feature
    triage -->|emits| develop
    triage -->|emits| feature
```
