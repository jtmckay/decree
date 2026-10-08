# llm_router

Ask a local language model, held to the request's reply_schema, to pick one of the options in the request.

Machine: [machines/llm_router.yml](../machines/llm_router.yml)

```mermaid
stateDiagram-v2
    [*] --> ask
    ask --> done: done
    ask --> failed: error (implicit)
    done --> [*]
    failed --> [*]
```
