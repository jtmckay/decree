# develop_by_size

Size up a change with a local classifier, implement it with a local model or with Claude, and verify it.

Machine: [machines/develop_by_size.yml](../machines/develop_by_size.yml)

```mermaid
stateDiagram-v2
    [*] --> describe
    describe --> size_up: done
    describe --> failed: error (implicit)
    implement_claude --> verify: done
    implement_claude --> failed: error (implicit)
    implement_claude --> failed: stop
    implement_local --> verify: done
    implement_local --> implement_claude: error
    size_up --> implement_claude: error
    size_up --> implement_claude: large (model: gliner_router)
    size_up --> implement_local: small (model: gliner_router)
    size_up --> implement_claude: unsure (model: gliner_router)
    tried_claude --> failed: false (check)
    tried_claude --> implement_claude: true (check)
    verify --> done: done
    verify --> tried_claude: error
    done --> [*]
    failed --> [*]
    note right of size_up
        model: gliner_router, min_confidence 0.7
    end note
    note right of tried_claude
        check: visits implement_claude less_than 1
    end note
```
