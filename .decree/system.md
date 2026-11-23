# All machines

Every machine, the `emits` and `invokes` edges between them, and cron entry points.

- [decree_develop](machines/decree_develop/graph.md): Implement a message with the AI in small, logged steps, run the project's gate, have the AI fix failures only if the gate fails, then have the AI check the acceptance criteria.
- [decree_infra](machines/decree_infra/graph.md): Add to the environment the project needs, the first time and every time after; the AI declares each resource in the machine's scripts/requirements (a project's plugin that extends decree_infra), then apply installs what is missing and checks every line.
- [decree_order](machines/decree_order/graph.md): At daemon start, run each machine's order/ files one at a time in file-name order, once each, and again when a file changes.
- [decree_qa](machines/decree_qa/graph.md): Attack the project to make it misbehave, and hand each real bug to develop as a failing test; the AI writes tests only and never fixes code.
- [decree_router](machines/decree_router/graph.md): Ask an AI CLI (claude, opencode or copilot) to pick one of the options in the request.
- [decree_skill](machines/decree_skill/graph.md): Keep the decree skill in the project, .claude/skills/decree/ or .github/skills/decree/ (or each folder scripts/dirs lists), the one this decree ships, each time the daemon starts.
- [develop](machines/develop/graph.md): Implement a message with the AI in small, logged steps, run the project's gate, have the AI fix failures only if the gate fails, then have the AI check the acceptance criteria.
- [infra](machines/infra/graph.md): Add to the environment the project needs, the first time and every time after; the AI declares each resource in the machine's scripts/requirements (a project's plugin that extends decree_infra), then apply installs what is missing and checks every line.
- [qa](machines/qa/graph.md): Attack the project to make it misbehave, and hand each real bug to develop as a failing test; the AI writes tests only and never fixes code.

```mermaid
flowchart LR
    decree_develop["decree_develop"]
    decree_infra["decree_infra"]
    decree_order["decree_order"]
    decree_qa["decree_qa"]
    decree_router["decree_router"]
    decree_skill["decree_skill"]
    develop["develop"]
    infra["infra"]
    qa["qa"]
    cron__decree_order_boot[/"cron: decree_order/boot"/]
    cron__decree_skill_boot[/"cron: decree_skill/boot"/]
    cron__decree_order_boot -->|cron| decree_order
    cron__decree_skill_boot -->|cron| decree_skill
    decree_qa -->|emits| decree_develop
    qa -->|emits| develop
```
