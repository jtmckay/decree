# financial_model

Third step of a business evaluation. Build projections, unit economics and funding needs from the prior analyses, then hand off to executive_summary.

Machine: [machines/financial_model.yml](../machines/financial_model.yml)

```mermaid
stateDiagram-v2
    [*] --> precheck
    analyze --> check_report: done
    analyze --> failed: error (implicit)
    check_report --> hand_off: done
    check_report --> failed: error (implicit)
    hand_off --> done: done
    hand_off --> failed: error (implicit)
    precheck --> analyze: done
    precheck --> failed: error (implicit)
    done --> [*]
    failed --> [*]
```
