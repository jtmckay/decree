---
routine: rust-develop
---

# 03: Embedded AI — REPL, Sessions, and Bench

## Overview

`decree ai` and `decree bench` exclusively use the **embedded Qwen 2.5
1.5B-Instruct** model via `llama-cpp-2`. They never delegate to external AI
providers (Claude CLI, Copilot, etc.) — those are only used by the
`commands.planning` and `commands.router` slots. Both commands share model
loading, hardware detection, and the GGUF download flow.

## Requirements

### Embedded Model Only

Both commands always load and run the GGUF model specified in `ai.model_path`
(default: `~/.decree/models/qwen2.5-1.5b-instruct-q5_k_m.gguf`). All
inference runs in-process via the `llama-cpp-2` Rust crate — no external
tools (`llama-cli`, `llama-server`, etc.) are needed. The decree binary is
fully self-contained for embedded AI.

If the model file is missing or `ai.model_path` is unset, both commands
offer to download the model (same built-in HTTP download as `decree init`)
and exit if the user declines — they never fall back to an external provider.

After initializing the `LlamaBackend`, call `backend.void_logs()` to
suppress llama.cpp's internal logging (KV cache layer info, tensor
diagnostics, etc.). Only decree's own output should be visible.
`decree bench --verbose` skips this suppression so the full llama.cpp
logs are accessible for debugging.

### Token Decoding

Token-to-string conversion **must** use `llama-cpp-2`'s
`token_to_piece` API (not the deprecated `tokens_to_str` /
`token_to_str` which use a fixed 8-byte buffer and fail on longer
token pieces). Both `decree ai` and `decree bench` share the same
decoding function.

### `decree ai` — Interactive REPL and One-Shot

The interface mirrors copilot and claude:
- `decree ai` — enters interactive REPL, creates a new session
- `decree ai --resume` — resumes the most recent session
- `decree ai --resume SESSION_ID` — resumes a specific session
- `decree ai -p "prompt"` — one-shot, runs prompt, prints output, exits
- Piped stdin supported in both modes

#### Interactive REPL (`decree ai` with no args)

- Print a welcome banner: `decree ai — interactive mode (type 'exit' or Ctrl-D to quit)`
- Print session ID: `session: {session_id}`
- Display `> ` prompt with context usage indicator (see below)
- Read user input line by line
- Generate response via the embedded model
- Print response, blank line, repeat
- Exit on "exit", "quit", EOF (Ctrl-D)

#### Session Persistence

Conversation history is persisted to disk so sessions survive across
process restarts. Each REPL session gets a unique session ID and is
saved as a YAML file under `.decree/sessions/`.

##### Session File Format

```yaml
# .decree/sessions/{session_id}.yml
id: "20260226143200"
created: "2026-02-26T14:32:00Z"
updated: "2026-02-26T14:35:12Z"
history:
  - role: user
    content: "What is Rust?"
  - role: assistant
    content: "Rust is a systems programming language..."
  - role: user
    content: "How does its borrow checker work?"
  - role: assistant
    content: "The borrow checker enforces..."
```

##### Session ID

Session IDs use the same timestamp format as message chains:
`YYYYMMDDHHmmss` (14 digits). Generated from the current time when a
new session starts.

##### Session Lifecycle

1. **New session** (`decree ai`): Generate a session ID, create an empty
   history, print "session: {id}".
2. **After each assistant response**: Save the full history to
   `.decree/sessions/{id}.yml`. This is an atomic write (write to
   `.tmp`, then rename) to prevent corruption on crash.
3. **Resume** (`decree ai --resume`): Load the session file, restore
   the history vector, print "resuming session: {id}", then enter the
   REPL with the existing history. The context usage indicator
   reflects the loaded history.
4. **Resume latest** (`decree ai --resume` with no ID): Find the most
   recently modified `.yml` file in `.decree/sessions/`, load it. If
   no sessions exist, print an error and exit.
5. **Resume by ID** (`decree ai --resume SESSION_ID`): Load the
   specific session file. If it does not exist, print an error listing
   available sessions and exit.

##### Init Creates Sessions Directory

`decree init` creates `.decree/sessions/` alongside the other
directories. The `.gitignore` template includes `sessions/` to keep
session files out of version control.

##### Session Data is the Full History

The session file stores the **complete** history — every user and
assistant message ever exchanged, including messages that have been
truncated from the in-memory context window. This means:

- The file grows monotonically (append-only semantically).
- On resume, the full history is loaded but then truncated to fit the
  context window before the first prompt. The user sees the truncation
  notice if needed.
- The file serves as a durable log of the conversation even after
  context truncation has dropped old messages from the model's view.

#### Conversation History (In-Memory)

The REPL maintains a working copy of the conversation history using the
standard chat message format: alternating `user` and `assistant` roles.
Each turn is appended to the history and the full history is sent to the
model on every prompt, so the model sees the entire conversation and can
reference earlier exchanges.

The in-memory history is a list of `(role, content)` pairs:

```
[
  { role: "user",      content: "What is Rust?" },
  { role: "assistant", content: "Rust is a systems programming language..." },
  { role: "user",      content: "How does its borrow checker work?" },
  { role: "assistant", content: "The borrow checker enforces..." },
]
```

On each turn:
1. Append the user's input as a `user` message
2. Truncate history if needed (see Context Truncation)
3. Build the full prompt by concatenating the history using the model's
   chat template (Qwen uses ChatML: `<|im_start|>role\ncontent<|im_end|>`)
4. Clear the KV cache (`ctx.clear_kv_cache()`)
5. Send the concatenated prompt to the model for generation
6. Append the model's response as an `assistant` message
7. Save the session file to disk

One-shot mode (`-p`) does **not** maintain history or create a session
— it is a single user->assistant exchange.

#### Context Window Usage Indicator

The prompt displays the percentage of the model's context window currently
occupied by the conversation history:

```
[14%] > what does the borrow checker do?
```

The percentage is calculated as:
`(tokens_in_history / context_window_size) * 100`, rounded to the nearest
integer. `context_window_size` defaults to 4096. `tokens_in_history` is
the token count of the full chat-templated history (all prior user and
assistant messages).

At the start of a fresh session the indicator reads `[0%]`.

#### Context Truncation

When the conversation history plus the new user message would exceed the
context window (leaving insufficient room for generation), the oldest
messages are dropped to make space:

1. Reserve space for generation: `max_tokens` (default: 4096 or the
   `--max-tokens` override) tokens are reserved for the model's response.
2. The available budget is `context_window_size - reserved_generation`.
3. Starting from the **oldest** message in history, drop messages (always
   in complete user+assistant pairs) until the remaining history plus the
   new user message fits within the budget.
4. If the history has fewer than 2 messages and still exceeds budget,
   warn: `"warning: input exceeds context window, truncating"` and break.
5. When messages are dropped, print a notice before the prompt:

```
~ context: dropped 4 earliest messages (history exceeded context window)
[87%] > continue explaining lifetimes
```

The notice is printed once per truncation event, not repeated on subsequent
turns unless more messages are dropped.

After truncation the percentage reflects the new (trimmed) history plus the
current message — it will typically be near but below 100%.

Note: truncation only affects the in-memory working history sent to the
model. The session file on disk retains the complete history.

#### One-Shot Mode (`decree ai -p "prompt"`)

- Generate response for the given prompt
- Print to stdout
- Exit with code 0
- No session file is created

#### Piped Input

- `echo "text" | decree ai` — use stdin as prompt (no REPL, no session)
- `cat file | decree ai -p "Summarize"` — `-p` becomes system prompt, stdin becomes user content

### Full CLI

`decree help` should display this reference.

```
decree                                         # Smart default
decree init [--model-path PATH]                # Initialize project

decree plan [PLAN]                             # Interactive planning
decree run [-m NAME] [-p PROMPT] [-v K=V ...]   # Run a message
decree run                                     # Interactive mode (TTY only)
decree process                                 # Batch: all unprocessed specs
decree daemon [--interval SECS]                # Daemon: monitor inbox + cron

decree diff                                    # Latest message's changes
decree diff <ID>                               # Specific message or chain
decree diff --since <ID>                       # From message onward

decree apply                                   # List available messages
decree apply <ID>                              # Apply message or chain
decree apply --through <ID>                    # All messages up through ID
decree apply --since <ID>                      # All messages from ID onward
decree apply --all                             # Apply everything
decree apply <ID> --force                      # Skip conflict check

decree sow                                     # Derive SOW from specs
decree ai [-p PROMPT] [--json] [--max-tokens N] # New session
decree ai --resume [SESSION_ID]                # Resume session
decree bench [PROMPT] [--runs N] [--max-tokens N] [--ctx N] [-v] # Benchmark
decree status                                  # Show progress
decree log [ID]                                # Show execution log
```

### `decree bench` — LLM Benchmark

#### Hardware / Backend Detection

Before benchmarking, detect and display the active compute backend:

- **GPU (Vulkan)**: detected when built with `--features vulkan` and a
  Vulkan-capable device is available at runtime. Report device name if
  obtainable (e.g. "NVIDIA GeForce RTX 4090", "AMD Radeon RX 7900").
- **GPU (CUDA)**: detected when built with `--features cuda` and CUDA
  runtime is available. Report device name.
- **GPU (Metal)**: detected when built with `--features metal` on macOS.
- **CPU**: fallback when no GPU backend is active, or when
  `ai.n_gpu_layers` is 0. Report "none (CPU-only build)" for the GPU line.

#### Output Format

Compact, table-based format with a header block, run table, output sample,
and warm-run summary:

```
────────────────────────────────────────────────────────────
Model:    /home/user/.decree/models/qwen2.5-1.5b-instruct-q5_k_m.gguf
Build:    CPU
GPU:      none (CPU-only build)
Ctx:      4096 tokens
Prompt:   "The answer to life, the universe, and everything i..." (~12 tokens)
Max gen:  200 tokens / run

   run      init   prefill       gen       tok/s
     1     1.28s     0.42s     6.91s       29.0  <- cold (model loading included)
     2     0.00s     0.42s     6.70s       29.8
     3     0.00s     0.42s     6.63s       30.2

Output:   "The answer to the question..."

avg prefill (warm): 0.42s   avg tok/s (warm): 30.0
────────────────────────────────────────────────────────────
```

#### Header Block

- **Model**: absolute path to the GGUF file
- **Build**: compile-time backend — one of `CPU`, `Vulkan`, `CUDA`, `Metal`
- **GPU**: device name if GPU backend is active, otherwise
  `none (CPU-only build)` or `none (n_gpu_layers = 0)`
- **Ctx**: context window size in tokens
- **Prompt**: the prompt text, truncated to ~50 chars with `...` if longer,
  followed by approximate token count in parens
- **Max gen**: maximum tokens to generate per run

#### Run Table

Fixed-width columns, right-aligned numbers:

| Column    | Description |
|-----------|-------------|
| `run`     | Run number (1-indexed) |
| `init`    | Model initialization time. Run 1 = cold load. Runs 2+ = 0.00s |
| `prefill` | Prompt evaluation time (processing input tokens) |
| `gen`     | Token generation (decode) time |
| `tok/s`   | Generation speed: `generated_tokens / gen_time` |

Run 1 gets a `<- cold (model loading included)` annotation.

#### Output Sample

After the run table, show a truncated sample of the generated text (first
~80 characters with `...` if longer).

#### Summary Line

A single line with warm-run averages (excluding run 1):
```
avg prefill (warm): 0.42s   avg tok/s (warm): 30.0
```

If `--runs 1`, there are no warm runs, so the summary line and cold
annotation are omitted.

#### Benchmark Metrics

Each run measures:
1. **Init time** — wall-clock time for model initialization. Run 1 loads
   from disk (cold). Runs 2+ reuse the loaded model (warm, ~0s).
2. **Prefill time** — time to process the input prompt tokens.
3. **Generation time** — time to generate output tokens.
4. **tok/s** — generation throughput: `generated_tokens / gen_time`.

#### Defaults

- **Runs**: 3 (one cold + two warm for meaningful averages)
- **Max tokens**: 200
- **Context size**: 4096
- **Prompt**: a built-in default (e.g. "The answer to life, the universe,
  and everything is")

## Acceptance Criteria

### Session Persistence

- **Given** a user starts a new REPL session with `decree ai`
  **When** the session begins
  **Then** a new session file is created in `.decree/sessions/` with a timestamp ID
  **And** the banner prints the session ID: `session: {id}`

- **Given** a REPL session is active
  **When** the user sends a message and receives a response
  **Then** the session file is updated with the new user and assistant messages
  **And** the write is atomic (write to .tmp, then rename)

- **Given** a previous session exists
  **When** the user runs `decree ai --resume`
  **Then** the most recently modified session is loaded
  **And** the banner prints `resuming session: {id}`
  **And** the context usage indicator reflects the loaded history

- **Given** a previous session with ID "20260226143200" exists
  **When** the user runs `decree ai --resume 20260226143200`
  **Then** that specific session is loaded and resumed

- **Given** no previous sessions exist
  **When** the user runs `decree ai --resume`
  **Then** an error is printed: `no sessions found in .decree/sessions/`
  **And** the command exits with a non-zero code

- **Given** a session ID that does not exist
  **When** the user runs `decree ai --resume BADID`
  **Then** an error is printed listing available sessions
  **And** the command exits with a non-zero code

- **Given** a session with 10 messages that was previously truncated
  **When** the session is resumed
  **Then** the full 10-message history is loaded from the file
  **And** context truncation re-applies as needed for the model's context window
  **And** the session file still contains all 10 messages

- **Given** `decree ai -p "prompt"` (one-shot mode) is used
  **When** the command completes
  **Then** no session file is created

- **Given** piped input is used (`echo "text" | decree ai`)
  **When** the command completes
  **Then** no session file is created

### Session File Format

- **Given** a session file exists
  **When** it is read
  **Then** it is valid YAML with fields: `id` (string), `created` (string),
  `updated` (string), `history` (array of {role, content} objects)

- **Given** a session with two exchanges
  **When** the file is read
  **Then** the history array has exactly 4 entries (2 user + 2 assistant)
  **And** roles alternate: user, assistant, user, assistant

### `decree ai` — REPL

- **Given** stdin is a TTY and no `-p` flag is provided
  **When** the user runs `decree ai`
  **Then** an interactive REPL starts with a welcome banner and `> ` prompt

- **Given** a prompt string is provided
  **When** the user runs `decree ai -p "What is Rust?"`
  **Then** the model generates a response, prints it to stdout, and exits with code 0

- **Given** text is piped to stdin and no `-p` flag is given
  **When** the user runs `echo "text" | decree ai`
  **Then** the piped text is used as the prompt (no REPL)

- **Given** text is piped to stdin and `-p` is also provided
  **When** the user runs `cat file | decree ai -p "Summarize"`
  **Then** `-p` is used as the system prompt and stdin as the user content

- **Given** the user is in an interactive REPL session
  **When** the user sends multiple messages
  **Then** the model receives the full conversation history with each prompt
  **And** the model can reference earlier exchanges in its responses

- **Given** the REPL session is active
  **When** the prompt is displayed
  **Then** it shows the context usage percentage: `[N%] > `

- **Given** the conversation history is empty (fresh session)
  **When** the first prompt is displayed
  **Then** the indicator reads `[0%] > `

- **Given** the conversation history plus the new user message exceeds the context window
  **When** the user sends a message
  **Then** the oldest complete user+assistant pairs are dropped until the history fits
  **And** a notice is printed: `~ context: dropped N earliest messages (history exceeded context window)`
  **And** the context percentage reflects the trimmed history

- **Given** the user runs `decree ai -p "prompt"` (one-shot mode)
  **When** the model generates a response
  **Then** no conversation history is maintained and no context indicator is shown

- **Given** the `--json` flag is passed
  **When** the model generates a response
  **Then** the output is constrained to valid JSON

- **Given** the `--max-tokens` flag is passed with a value
  **When** the model generates a response
  **Then** generation respects the token limit

### Token Decoding

- **Given** a sequence of generated tokens including tokens whose UTF-8
  representation exceeds 8 bytes
  **When** the tokens are decoded to text
  **Then** the correct string is returned without errors

### `decree bench`

- **Given** a valid model is configured
  **When** the user runs `decree bench`
  **Then** the benchmark runs 3 times with a built-in prompt, printing
  header block, run table, output sample, and warm summary

- **Given** the user provides a custom prompt
  **When** running `decree bench "Explain quicksort"`
  **Then** the custom prompt is used and shown (truncated) in the header

- **Given** `--runs 5` is passed
  **When** the benchmark executes
  **Then** five runs are performed, run 1 is cold, warm summary averages runs 2-5

- **Given** `--runs 1` is passed
  **When** the benchmark executes
  **Then** one run with no cold/warm annotation and no warm summary line

- **Given** `--max-tokens` is passed
  **When** the benchmark generates tokens
  **Then** generation stops at the specified limit

- **Given** the build includes `--features vulkan` and a GPU is available
  **When** `decree bench` runs
  **Then** Build shows "Vulkan" and GPU shows the device name

- **Given** no GPU feature is enabled or `n_gpu_layers` is 0
  **When** `decree bench` runs
  **Then** Build shows "CPU" and GPU shows "none (CPU-only build)"

### Init Integration

- **Given** `decree init` is run
  **When** the directory structure is created
  **Then** `.decree/sessions/` exists
  **And** `.decree/.gitignore` contains `sessions/`

### Shared

- **Given** no model is configured or the model file is missing
  **When** the user runs `decree ai` or `decree bench`
  **Then** it offers to download the model using the built-in HTTP client
  **And** if the user declines, prints the manual download URL and exits
  **And** the command does not fall back to any external AI provider

### CLI Integration

- **Given** a user runs `decree ai --help`
  **When** help is displayed
  **Then** it shows `--resume`, `--json`, `--max-tokens`, and `-p` options

- **Given** a user runs `decree bench --help`
  **When** help is displayed
  **Then** it shows `--runs`, `--max-tokens`, `--ctx`, and `-v` options

- **Given** `decree ai` is run outside a decree project
  **When** the command executes
  **Then** it fails with "not a decree project" error

- **Given** `decree bench` is run outside a decree project
  **When** the command executes
  **Then** it fails with "not a decree project" error
