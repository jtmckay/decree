# All machines

Every machine, the `emits` and `invokes` edges between them, and cron entry points.

- [develop](develop.md): Implement a message with Claude in small, logged steps, run the project's gate, have Claude fix failures only if the gate fails, then have Claude check the acceptance criteria.
- [infra](infra.md): Add to the environment the project needs, the first time and every time after; Claude declares each resource in lib/infra/requirements, then apply installs what is missing and checks every line.
- [router](router.md): Ask Claude to pick one of the options in the request.

```mermaid
flowchart LR
    develop["develop"]
    infra["infra"]
    router["router"]
```
