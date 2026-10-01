# hello

Run one script.

```mermaid
stateDiagram-v2
    [*] --> greet
    greet --> done: done
    greet --> failed: error (implicit)
    done --> [*]
    failed --> [*]
```
