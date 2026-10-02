# All machines

Every machine, the `emits` edges between them, and cron entry points.

```mermaid
flowchart LR
    claude_router["claude_router"]
    deploy["deploy"]
    develop["develop"]
    feature["feature"]
    hello["hello"]
    ship["ship"]
    triage["triage"]
    cron__nightly_audit[/"cron: nightly-audit"/]
    cron__nightly_audit -->|cron| develop
    feature -->|emits| feature
    triage -->|emits| develop
    triage -->|emits| feature
    feature -->|invokes| claude_router
    ship -->|invokes| deploy
    ship -->|invokes| feature
    triage -->|invokes| claude_router
```
