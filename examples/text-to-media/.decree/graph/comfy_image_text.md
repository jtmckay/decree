# comfy_image_text

Generate an image from a text prompt with the ComfyUI FLUX2 workflow.

Machine: [machines/comfy_image_text.yml](../machines/comfy_image_text.yml)

```mermaid
stateDiagram-v2
    [*] --> precheck
    build_payload --> queue_prompt: done
    build_payload --> failed: error (implicit)
    precheck --> build_payload: done
    precheck --> failed: error (implicit)
    queue_prompt --> done: done
    queue_prompt --> failed: error (implicit)
    done --> [*]
    failed --> [*]
```
