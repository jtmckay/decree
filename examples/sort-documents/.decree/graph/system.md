# All machines

Every machine, the `emits` and `invokes` edges between them, and cron entry points.

- [gliner_router](gliner_router.md): Ask the local GLiNER2.5-Decide classifier to pick one of the options in the request.
- [router](router.md): Ask Claude to pick one of the options in the request.
- [sort_document](sort_document.md): File one scanned document as an invoice, a receipt or other paperwork.

```mermaid
flowchart LR
    gliner_router["gliner_router"]
    router["router"]
    sort_document["sort_document"]
    sort_document -->|invokes| gliner_router
    sort_document -->|invokes| router
```
