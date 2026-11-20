# deploy

Build, ask a person to approve, then ship.

Machine: [machines/deploy/machine.yml](machine.yml)

```mermaid
stateDiagram-v2
    [*] --> build
    approval --> ship: approve (reply)
    approval --> failed: error (implicit)
    approval --> rejected: reject (reply)
    build --> approval: done
    build --> failed: error (implicit)
    ship --> done: done
    ship --> failed: error (implicit)
    done --> [*]
    failed --> [*]
    rejected --> [*]
    note right of approval
        reply: ask_person
    end note
```
