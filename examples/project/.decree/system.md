# All machines

Every machine, the `emits` and `invokes` edges between them, and cron entry points.

- [decree_order](machines/decree_order/graph.md): At daemon start, run each machine's order/ files one at a time in file-name order, once each, and again when a file changes.
- [deploy](machines/deploy/graph.md): Build, ask a person to approve, then ship.
- [develop](machines/develop/graph.md): Implement a message, then test it.
- [hello](machines/hello/graph.md): Run one script.

```mermaid
flowchart LR
    decree_order["decree_order"]
    deploy["deploy"]
    develop["develop"]
    hello["hello"]
    cron__decree_order_boot[/"cron: decree_order/boot"/]
    cron__develop_nightly_audit[/"cron: develop/nightly-audit"/]
    cron__decree_order_boot -->|cron| decree_order
    cron__develop_nightly_audit -->|cron| develop
```
