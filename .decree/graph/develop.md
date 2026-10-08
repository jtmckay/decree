# develop

Implement a message with Claude in small, logged steps, run the project's gate, have Claude fix failures only if the gate fails, then have Claude check the acceptance criteria.

Machine: [machines/develop.yml](../machines/develop.yml)

```mermaid
stateDiagram-v2
    [*] --> precheck
    final_gate --> failed: error (implicit)
    final_gate --> failed: fail
    final_gate --> verify: pass
    fix --> final_gate: done
    fix --> failed: error (implicit)
    fix --> failed: stop
    gate --> failed: error (implicit)
    gate --> fix: fail
    gate --> verify: pass
    implement --> gate: done
    implement --> failed: error (implicit)
    implement --> failed: stop
    precheck --> implement: done
    precheck --> failed: error (implicit)
    verify --> failed: error (implicit)
    verify --> failed: fail
    verify --> done: pass
    verify --> failed: stop
    done --> [*]
    failed --> [*]
```
