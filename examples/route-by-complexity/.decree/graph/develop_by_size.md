# develop_by_size

Size up a change with a local classifier, then implement and test it, locally first or with Claude only, moving to Claude when local attempts fail the tests.

Machine: [machines/develop_by_size.yml](../machines/develop_by_size.yml)

```mermaid
stateDiagram-v2
    [*] --> describe
    claude_only --> done: done
    claude_only --> failed: error (implicit)
    claude_only --> failed: stop
    describe --> size_up: done
    describe --> failed: error (implicit)
    local_first --> done: done
    local_first --> failed: error (implicit)
    local_first --> failed: stop
    size_up --> claude_only: error
    size_up --> claude_only: large (model: gliner_router)
    size_up --> local_first: small (model: gliner_router)
    size_up --> claude_only: unsure (model: gliner_router)
    done --> [*]
    failed --> [*]
    note right of claude_only
        attempts: claude → claude
    end note
    note right of local_first
        attempts: local → local → claude
    end note
    note right of size_up
        model: gliner_router, min_confidence 0.7
    end note
```
