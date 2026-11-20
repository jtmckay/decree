# 05: Routine CLI and Pre-Check Verification

## Overview

`decree routine` is the interactive way to run routines. It walks the
user through selecting a routine, reviewing its description and
pre-check status, then prompting for each input step by step. Both
`decree routine` (no args) and `decree routine <name>` lead to the
same guided flow. `decree verify` runs all pre-checks at once.

## `decree routine` — Interactive Guided Run

Both forms lead to the same guided flow:

```
decree routine                # start with routine selection
decree routine <name>         # skip selection, start at description
```

### Step 1: Select Routine (if no name given)

Arrow-key selector with fuzzy type-ahead filtering. Config default
routine is pre-highlighted.

```
Select a routine:
> develop          Default routine that delegates work to an AI assistant
  rust-develop     Rust-specific development routine with cargo build/test
  deploy/staging   Deploy to staging environment
  transcribe       Transcribes audio using OpenAI Whisper
```

### Step 2: Show Description and Pre-Check

Print the full routine description and run the pre-check:

```
transcribe (.decree/routines/transcribe.sh)

  Transcribes audio using OpenAI Whisper via the command line.
  Supports multiple models and output formats.

  Pre-check: PASS
```

If pre-check fails, show the failure reason and ask to continue:

```
  Pre-check: FAIL: whisper not found

  Continue anyway? [y/N]
```

### Step 3: Prompt for Custom Parameters

For each custom parameter discovered in the routine, prompt with the
parameter name and its default value:

```
output_file [default: ""]:
>

model [default: "large"]:
>
```

- Show `[default: "value"]` for each parameter
- Empty default (`""`) means optional — pressing Enter skips
- Non-empty default means pressing Enter uses that default
- Parameters with empty defaults that are essential to the routine
  should be documented in the routine's description (decree does not
  enforce required custom params — that's the routine's responsibility)

### Step 4: Message Body

```
Message body [recommended, empty line to submit]:
>
```

Multi-line input: the user types lines until they enter an empty line
(just press Enter on a blank line) to submit. The body can be empty —
pressing Enter immediately on the first line submits an empty body.

### Step 5: Execute

After all inputs are collected:
1. Generate a new chain ID
2. Create an inbox message with the collected frontmatter and body
3. Run the pre-check (if not already passed in step 2)
4. Process the message immediately through the standard pipeline

Display a summary before executing:

```
Running transcribe:
  output_file: ./transcripts/meeting.txt
  model: large
  body: "Transcribe this meeting recording."

Press Enter to run, Ctrl-C to cancel.
```

## Non-TTY Mode

When stdin is not a TTY, `decree routine` prints the routine list and
exits. `decree routine <name>` prints the detail view and exits.

## Error: Unknown Routine

Uses fuzzy matching (Levenshtein distance or similar). If a close match
is found (distance <= 3):

```
Error: unknown routine 'devlop'

Did you mean 'develop'?
```

If no close match, list available routines:

```
Error: unknown routine 'foo'

Available routines:
  develop          Default routine that delegates work to an AI assistant
  rust-develop     Rust-specific development routine with cargo build/test
```

## Error: No Routines

```
No routines found in .decree/routines/
```

## `decree verify` — Run All Pre-Checks

Runs pre-checks for every discovered routine and reports results:

```
$ decree verify

Routine pre-checks:
  develop          PASS
  rust-develop     PASS
  deploy/staging   FAIL: kubectl not found
  transcribe       FAIL: whisper not found

2 of 4 routines ready.
```

Exit code: 0 if all pass, 3 if any fail.

## Discovery Rules

### Description Extraction

1. Skip the shebang (`#!/...`)
2. Next comment line is the title (e.g. `# Transcribe`)
3. Skip `#`-only blank comment lines
4. Collect subsequent comment lines, stripping `# `
5. First line is the short description (list view)
6. Full block is the long description (detail view)

### Custom Parameter Discovery

1. Skip shebang, comments, blanks, `set` builtins, pre-check block
2. Match `var="${var:-default}"` assignments
3. Stop at first non-matching line
4. Exclude standard params (`message_file`, `message_id`, etc.)
5. Remainder are custom parameters with defaults from `:-default`

## Acceptance Criteria

- [ ] `decree routine` (no args, TTY) shows arrow-key selector
- [ ] `decree routine <name>` skips selection, starts at description
- [ ] Full description is printed from comment header
- [ ] Pre-check runs and shows PASS or FAIL with reason
- [ ] Failed pre-check asks to continue with `[y/N]`
- [ ] Each custom parameter prompts with name and `[default: "value"]`
- [ ] Empty Enter on custom param uses the default value
- [ ] Message body accepts multi-line input, empty line submits
- [ ] Empty message body is accepted
- [ ] Body shows `[recommended]`
- [ ] Summary is shown before execution with all collected values
- [ ] Enter confirms execution, Ctrl-C cancels
- [ ] Message is created and processed through standard pipeline
- [ ] Non-TTY: `decree routine` prints list and exits
- [ ] Non-TTY: `decree routine <name>` prints detail and exits
- [ ] Unknown routine with close match shows "Did you mean?" suggestion
- [ ] Unknown routine with no close match shows available routines list
- [ ] `decree verify` runs all pre-checks and reports summary
- [ ] `decree verify` exits 0 when all pass, 3 when any fail
- [ ] Nested directory routines are discovered and shown correctly
