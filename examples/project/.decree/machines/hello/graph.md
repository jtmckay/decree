# hello

Run one script.

Machine: [machines/hello/machine.yml](machine.yml)

```mermaid
stateDiagram-v2
    [*] --> greet
    greet --> done: done
    greet --> failed: error (implicit)
    done --> [*]
    failed --> [*]
```
