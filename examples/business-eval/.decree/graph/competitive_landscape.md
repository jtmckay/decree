# competitive_landscape

Second step of a business evaluation. Map competitors and positioning, building on the market analysis, then hand off to financial_model.

Machine: [machines/competitive_landscape.yml](../machines/competitive_landscape.yml)

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
