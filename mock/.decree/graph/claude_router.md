# claude_router

Ask Claude to pick one of the options in the request.

Machine: [machines/claude_router.yml](../machines/claude_router.yml)

```mermaid
stateDiagram-v2
    [*] --> ask
    ask --> done: done
    ask --> failed: error (implicit)
    done --> [*]
    failed --> [*]
```
