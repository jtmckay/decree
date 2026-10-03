# transcribe

Transcribe an audio file with OpenAI Whisper into a .txt file.

Machine: [machines/transcribe.yml](../machines/transcribe.yml)

```mermaid
stateDiagram-v2
    [*] --> precheck
    precheck --> transcribe: done
    precheck --> failed: error (implicit)
    transcribe --> done: done
    transcribe --> failed: error (implicit)
    done --> [*]
    failed --> [*]
```
