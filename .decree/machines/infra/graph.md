# infra

Add to the environment the project needs, the first time and every time after; the AI declares each resource in machines/infra/scripts/requirements, then apply installs what is missing and checks every line.

Machine: [machines/infra/machine.yml](machine.yml)

```mermaid
stateDiagram-v2
    [*] --> declare
    apply --> done: done
    apply --> failed: error (implicit)
    declare --> apply: done
    declare --> failed: error (implicit)
    declare --> failed: stop
    done --> [*]
    failed --> [*]
    note left of declare
        machine onentry: clean_tree
    end note
    note right of done
        onentry: commit
    end note
    note right of failed
        onentry: keep
    end note
```
