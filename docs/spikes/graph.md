# Spike: seeing the graph

**Status:** decided 2026-10-01 (decision record at the end). Spec section 9 and ticket M1.4 carry the result.

## Why

`decree graph` prints Mermaid text. That renders well on GitHub, GitLab, VS Code and Obsidian. A user in a terminal, outside an IDE or a website, sees only text, so decree needs a standard way to turn the graph into something to look at.

## Settled

- Mermaid `stateDiagram-v2` text stays the canonical output: it is diffable, testable byte for byte (`mock/graph/*.md`), and renders natively where most people read code.
- Whatever decree adds must work from the same arena, so the picture can never drift from the behaviour.

## Criteria

1. Shows what matters: nesting, notes (entry and exit actions), and who decides each edge (deterministic, model or human, R9 of the router spike).
2. Works offline, and sends nothing to a third party by default.
3. Adds little or nothing to install: decree is a single 3.5 MB binary today.
4. Opens in one command.

## Options

| # | Option | How the user sees it | Criteria notes |
| --- | --- | --- | --- |
| G1 | Mermaid text only (today) | Paste into GitHub, an IDE or mermaid.live. | No new code; fails criterion 4 in a terminal. |
| G2 | Shell out to an installed renderer: `mmdc` ([mermaid-cli](https://github.com/mermaid-js/mermaid-cli)) or `merman-cli` | `decree graph --svg out.svg` runs whichever is on `PATH`, else explains how to install one. | Reference-quality output. mermaid-cli needs Node and headless Chromium; merman-cli is a 35 MB static binary. Nothing added to decree. |
| G3 | Build a Rust renderer in: [merman](https://docs.rs/crate/merman/0.7.0) | `decree graph --svg out.svg` / `--png`. | Offline, no tools to install. merman's slices are 9 to 17 MB, so decree's binary would grow several times; beta. Could sit behind a cargo feature. |
| G4 | Self-contained HTML page | `decree graph --html out.html` (or `--open`) writes a page that loads mermaid.js and opens it with `xdg-open` / `open`. | Mermaid's own renderer, so best fidelity. From a CDN it needs network; embedding mermaid.js adds about 3 MB to decree. |
| G5 | A mermaid.live link | `decree graph --link` prints `https://mermaid.live/edit#pako:<zlib+base64url of the diagram>`. | Zero dependencies. The diagram stays in the URL fragment, which browsers do not send to the server, but the page itself loads from mermaid.live: not offline. |
| G6 | DOT for Graphviz | `decree graph --format dot \| dot -Tsvg > out.svg`. | Graphviz is a mature, common package, but compound states need clusters plus `compound=true` tricks, and every graph feature is written twice. Pure-Rust DOT renderers ([layout-rs](https://docs.rs/layout-rs)) do not support nested graphs. |
| G7 | XState JSON export | `decree graph --format xstate` for [Stately's visualizer](https://stately.ai/docs) or the XState VS Code extension. | Statechart-native view (entry and exit actions, nesting), and decree's machines are XState-shaped. Needs a browser or an editor; Stately is a hosted service. |
| G8 | Terminal rendering | ASCII or Unicode in the terminal. | Immature for state diagrams: merman's ASCII output does not support them, and the alternatives (termaid, meraid) are young. Not viable now. |

## Evidence so far

The two mock fixtures (`mock/graph/feature.md` with a nested compound state and notes, `mock/graph/system.md` with three machines) were rendered with three renderers. Images are in `docs/spikes/graph/`.

| Renderer | Time | Result |
| --- | --- | --- |
| mermaid-cli 11 (Docker `minlag/mermaid-cli`) | about 2 s plus browser start | Reference. Correct. |
| merman-cli 0.7.0 (Rust) | 0.04 s (feature), 0.09 s (system) | Matches the reference closely: nesting, notes on the right states, `#lt;` decoded, final states correct. |
| mmdr 0.3.1 (Rust, [mermaid-rs-renderer](https://github.com/1jehuang/mermaid-rs-renderer)) | 0.13 s | Not usable yet: notes detached and attached to the wrong states (`done`'s `commit` merged with `implement`'s `snapshot`), `#lt;` not decoded, final states drawn as bars, labels overlapping. |

So the Mermaid text format does not need to change, and a Rust-native renderer of good quality exists.

## Leaning

G2 plus G5 now; G3 later if needed.

- `decree graph --svg <file>` (and `--png`) shells out to `merman-cli` or `mmdc`, whichever is installed. That gives reference-quality images, keeps decree small, and is one command.
- `decree graph --link` for a zero-install view in the browser, documented as sending the page request to mermaid.live.
- Revisit building merman in (G3, behind a cargo feature) if installing a renderer proves to be a real obstacle.
- DOT (G6) stays deferred: the evidence shows no gap that DOT would fill.
- XState export (G7) is attractive for standards alignment but is a separate feature; record it as a follow-up.

## Method to finish

1. Add the router spike's R9 marks (`(choice: jev)`, `(human)`) to the fixtures and re-render with merman-cli and mermaid-cli.
2. Render a larger machine (about 20 states, two levels of nesting) to check that the layout stays readable.
3. Confirm the mermaid.live link format by round-tripping a fixture through the editor.
4. Decide, write the decision record below, and update spec section 9 and ticket M1.4.

## Decision record

| Decision | Reason |
| --- | --- |
| decree prints Mermaid text only. | Keep decree a small single binary; Mermaid is the standard with the widest viewer support. |
| The output is a Markdown document with the diagram in a `mermaid` fence. | Saved as `.md` it opens as a picture in VS Code 1.121+ (native Mermaid preview), GitHub, GitLab and Obsidian, with no copy and paste. |
| Users are told to paste it into https://mermaid.live (spec section 9, Viewing). | No install. Mermaid and the live editor are MIT, so teams can host their own. |
| No XState export (G7). | Stately Studio requires an account to view; that is not an acceptable default. |
| No image rendering in decree (G2, G3, G4), no DOT (G6), no terminal rendering (G8). | Rendering belongs to viewers; DOT would duplicate every graph feature. |
| A clickable graph linking to logs is a separate product built on decree, not part of decree. | decree's contract for it: stable state ids in the Mermaid output, plus `machine`, `state` and `run_id` on every event. |

Checked along the way: an XState export of the mock loaded in `xstate` 5.33.2 with every target resolving. Dropped for the account requirement, not for correctness.

The renders in `docs/spikes/graph/` were evidence for this decision. They can be deleted once the spike is reviewed.
