# decree_skill

Keep the decree skill in the project, .claude/skills/decree/ or .github/skills/decree/ (or each folder scripts/dirs lists), the one this decree ships, each time the daemon starts.

Machine: [machines/decree_skill/machine.yml](machine.yml)

```mermaid
stateDiagram-v2
    [*] --> install
    install --> done: done
    install --> failed: error (implicit)
    done --> [*]
    failed --> [*]
```
