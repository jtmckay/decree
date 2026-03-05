# 09: Prompts

## Overview

`decree prompt` loads a prompt template from `.decree/prompts/`,
substitutes any `{variable}` placeholders with project context, prints
it to the terminal, and offers two actions: copy to clipboard or launch
the configured interactive AI tool. The prompt template is the entire
prompt — decree does not inject any wrapper text or instructions.

All routines are non-interactive. The only interactive AI usage is
through `decree prompt`, which fully hands off terminal control to the
AI tool via exec.

## `decree prompt` — Interactive Guided Flow

Both forms lead to the same guided flow:

```
decree prompt               # start with prompt selection
decree prompt <name>         # skip selection, start at preview
```

Prompts are templates in `.decree/prompts/`. Each `.md` file is a
prompt template. The prompt name is the filename without `.md`.

### Step 1: Select Prompt (if no name given)

Arrow-key selector with fuzzy type-ahead filtering. Prompt names are
derived from filenames without `.md`.

```
Select a prompt:
> migration      Migration template for planning work
  sow            Statement of work template
  routine        Guide for building new routines
```

Prompt descriptions are the first non-blank line of the file content
(truncated to 60 characters for the list view).

### Step 2: Preview Prompt

Build the prompt by reading the template file and substituting any
`{variable}` placeholders (see "Variable Substitution" below). Print
the full prompt to stdout.

### Step 3: Action Prompt

```
Press C to copy to clipboard, or Enter to launch AI:
```

- **C**: copy the prompt to the system clipboard and exit.
  Use platform-appropriate clipboard command:
  - Linux: `xclip -selection clipboard` or `xsel --clipboard`
  - macOS: `pbcopy`
  - Windows/WSL: `clip.exe`
  Print: `Prompt copied to clipboard.`
- **Enter**: exec into `commands.ai_interactive` from config (e.g.
  `opencode`, `claude`). The AI process replaces decree and gets
  full terminal control.

Any other key or Ctrl-C exits without action.

## Variable Substitution

Prompt templates are plain markdown files. They may contain `{variable}`
placeholders that decree substitutes before display:

| Variable | Value |
|---|---|
| `{migrations}` | List of files in `.decree/migrations/` with titles, or "None yet" |
| `{routines}` | List of routines with descriptions from `.decree/routines/` |
| `{processed}` | Contents of `.decree/processed.md`, or "None yet" |
| `{config}` | Contents of `.decree/config.yml` |

Unknown `{variable}` placeholders are left as-is (not an error).
Templates without any placeholders are used verbatim.

## Non-TTY Mode

When stdin is not a TTY:
- `decree prompt` prints the prompt list and exits
- `decree prompt <name>` prints the substituted prompt to stdout and exits
  (useful for piping: `decree prompt migration | pbcopy`)

## Error: Unknown Prompt

Uses fuzzy matching (Levenshtein distance or similar). If a close match
is found (distance <= 3):

```
Error: unknown prompt 'migraton'

Did you mean 'migration'?
```

If no close match, list available prompts:

```
Error: unknown prompt 'foo'

Available prompts:
  migration      Migration template for planning work
  sow            Statement of work template
  routine        Guide for building new routines
```

## Config

```yaml
commands:
  ai_interactive: "opencode"    # Launched on Enter
```

Set during `decree init` based on the detected AI backend:
- opencode detected → `"opencode"`
- claude detected → `"claude"`
- copilot detected → `"copilot"`

This is the bare interactive command (no `{prompt}` placeholder),
distinct from `commands.ai_router` which is for routine selection.

## Acceptance Criteria

- [ ] `decree prompt` without args shows arrow-key selector of templates
- [ ] `decree prompt <name>` skips selection, enters same flow at preview step
- [ ] Invalid prompt name with close match shows "Did you mean?" suggestion
- [ ] Invalid prompt name with no close match shows available list
- [ ] Prompt template is the entire prompt — no wrapper text injected
- [ ] `{variable}` placeholders are substituted with project context
- [ ] Unknown placeholders are left as-is
- [ ] Templates without placeholders are used verbatim
- [ ] Prompt descriptions in selector are first non-blank line of file (truncated)
- [ ] Full prompt is printed to stdout
- [ ] Pressing C copies prompt to system clipboard and exits
- [ ] Pressing Enter execs into `commands.ai_interactive`
- [ ] Clipboard uses platform-appropriate command (xclip/pbcopy/clip.exe)
- [ ] Any other key or Ctrl-C exits without action
- [ ] `commands.ai_interactive` is set during init based on detected AI
- [ ] Non-TTY: `decree prompt` prints list and exits
- [ ] Non-TTY: `decree prompt <name>` prints substituted prompt and exits
- [ ] All routines are non-interactive — only `decree prompt` launches interactive AI
