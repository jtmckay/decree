# All machines

Every machine, the `emits` and `invokes` edges between them, and cron entry points.

- [build](machines/build/graph.md): Build one change.
- [plan](machines/plan/graph.md): Split a request into follow-up messages for build.

```mermaid
flowchart LR
    build["build"]
    plan["plan"]
    cron__plan_weekly_plan[/"cron: plan/weekly-plan"/]
    cron__plan_weekly_plan -->|cron| plan
    plan -->|emits| build
```
