# Rust conventions (develop's implement, fix and verify prompts read this)

- Format with `cargo fmt`; keep `cargo clippy --all-targets -- -D warnings` clean.
- Unit tests in a `#[cfg(test)] mod tests` beside the code; behaviour a user sees
  (commands, files, exit codes) in `tests/`, driving the built binary.
- Return errors with context; no `unwrap()` or `expect()` outside tests unless the
  invariant is proven on the line above.
- No new dependency unless the message allows it.
- `cargo test` must pass before you end; run the narrowest test first while working.
