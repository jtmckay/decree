# decree_gliner_router

Ask the local GLiNER2.5-Decide classifier to pick one of the options in the request.

Machine: [machines/decree_gliner_router/machine.yml](machine.yml)

```mermaid
stateDiagram-v2
    [*] --> ask
    ask --> done: done
    ask --> failed: error (implicit)
    done --> [*]
    failed --> [*]
```
