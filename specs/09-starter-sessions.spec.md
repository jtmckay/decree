# 09: Starter Prompts

## Overview

`decree starter` constructs a context-rich prompt from a starter
template and project state, prints it to the terminal, and offers
two actions: copy to clipboard or launch the configured interactive
AI tool. No tool-specific continuation logic — just a prompt you
can take anywhere.

## `decree starter` — Interactive Guided Flow

Both forms lead to the same guided flow:

```
decree starter               # start with starter selection
decree starter <name>         # skip selection, start at preview
```

Starters are templates in `.decree/starters/`. Each `.md` file is a
starter template. The starter name is the filename without `.md`.

### Step 1: Select Starter (if no name given)

Arrow-key selector with fuzzy type-ahead filtering.

```
Select a starter:
> spec           Spec/migration template for planning work
  bugfix         Template for bug fix workflows
```

### Step 2: Preview Prompt

Build the prompt from the template + project context (see "Prompt
Construction" below) and print the full prompt to stdout.

### Step 3: Action Prompt

```
Press Enter to launch interactive AI, or C to copy to clipboard:
```

- **Enter**: exec into `commands.interactive_ai` from config (e.g.
  `opencode`, `claude`). The AI process replaces decree and gets
  full terminal control. The user pastes/has the prompt context.
- **C**: copy the prompt to the system clipboard and exit.
  Use platform-appropriate clipboard command:
  - Linux: `xclip -selection clipboard` or `xsel --clipboard`
  - macOS: `pbcopy`
  - Windows/WSL: `clip.exe`
  Print: `Prompt copied to clipboard.`

Any other key or Ctrl-C exits without action.

## Prompt Construction

```
You are a planning assistant for a software project.

## Starter Template
{contents of .decree/starters/<selected>.md}

## Existing Migrations
{list of files in migrations/ with their titles, or "None yet"}

## Instructions
1. Analyse the request and existing project state.
2. Present a numbered plan summary with proposed migration files.
3. WAIT for approval — do NOT generate files until told to proceed.
4. When approved, generate each migration file:
   - Filename: NN-descriptive-name.md
   - Include YAML frontmatter with `routine:` field
   - Write each file to the migrations/ directory
```

## Config

```yaml
commands:
  interactive_ai: "opencode"    # Launched on Enter
```

Set during `decree init` based on the detected AI backend:
- opencode detected → `"opencode"`
- claude detected → `"claude"`
- copilot detected → `"copilot"`

This is the bare interactive command (no `{prompt}` placeholder),
distinct from `commands.ai` which is for non-interactive routine
execution.

## Output

After the user's AI session (however they choose to start it), the
AI produces:
- **Migration files** in `migrations/` with `NN-name.md` naming
- Or **inbox messages** in `.decree/inbox/` for direct processing

The user then runs `decree process` to execute the work.

## Acceptance Criteria

- [ ] `decree starter` without args shows arrow-key selector of templates
- [ ] `decree starter <name>` skips selection, enters same flow at preview step
- [ ] Invalid starter name shows error with available list
- [ ] Prompt includes selected template and existing migrations list
- [ ] Full prompt is printed to stdout
- [ ] Pressing Enter execs into `commands.interactive_ai`
- [ ] Pressing C copies prompt to system clipboard and exits
- [ ] Clipboard uses platform-appropriate command (xclip/pbcopy/clip.exe)
- [ ] Any other key or Ctrl-C exits without action
- [ ] `commands.interactive_ai` is set during init based on detected AI
