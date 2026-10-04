# local_router

Ask the local classifier to score the options in the request.

Machine: [machines/local_router.yml](../machines/local_router.yml)

```mermaid
stateDiagram-v2
    [*] --> ask
    ask --> done: done
    ask --> failed: error (implicit)
    done --> [*]
    failed --> [*]
```
