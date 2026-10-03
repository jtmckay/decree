# executive_summary

Last step of a business evaluation. Synthesize the prior analyses into a scorecard and a go/no-go recommendation.

Machine: [machines/executive_summary.yml](../machines/executive_summary.yml)

```mermaid
stateDiagram-v2
    [*] --> precheck
    analyze --> check_report: done
    analyze --> failed: error (implicit)
    check_report --> done: done
    check_report --> failed: error (implicit)
    precheck --> analyze: done
    precheck --> failed: error (implicit)
    done --> [*]
    failed --> [*]
```
