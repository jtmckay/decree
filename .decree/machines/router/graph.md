# router

Ask an AI CLI (claude, opencode or copilot) to pick one of the options in the request.

Machine: [machines/router/machine.yml](machine.yml)

```mermaid
stateDiagram-v2
    [*] --> ask
    ask --> done: done
    ask --> failed: error (implicit)
    done --> [*]
    failed --> [*]
```
