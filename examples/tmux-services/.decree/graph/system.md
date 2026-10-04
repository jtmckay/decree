# All machines

Every machine, the `emits` and `invokes` edges between them, and cron entry points.

- [gliner_router](gliner_router.md): Ask the local GLiNER2.5-Decide classifier to pick one of the options in the request.
- [illustrated_post](illustrated_post.md): Write a short post from the message, with a picture when the message asks for one.

```mermaid
flowchart LR
    gliner_router["gliner_router"]
    illustrated_post["illustrated_post"]
    illustrated_post -->|invokes| gliner_router
```
