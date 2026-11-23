# triage

Read a free-form request and hand it to the machine that should do the work.

Machine: [machines/triage/machine.yml](machine.yml)

```mermaid
stateDiagram-v2
    [*] --> classify
    classify --> failed: error (implicit)
    classify --> to_feature: feature (model)
    classify --> rejected: reject (model)
    classify --> to_develop: small_change (model)
    to_develop --> done: done
    to_develop --> failed: error (implicit)
    to_feature --> done: done
    to_feature --> failed: error (implicit)
    done --> [*]
    failed --> [*]
    rejected --> [*]
    note right of classify
        model: decree_router
    end note
```
