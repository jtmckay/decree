# market_analysis

First step of a business evaluation. Analyze the market for the idea in the message body, then hand off to competitive_landscape.

Machine: [machines/market_analysis.yml](../machines/market_analysis.yml)

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
