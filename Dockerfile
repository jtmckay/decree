# Build stage: compile decree
FROM rust:1-slim-bookworm AS builder

WORKDIR /build

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Build the real binary
COPY src/ src/
RUN touch src/main.rs && cargo build --release

# Runtime stage
FROM node:24-bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        bash git curl ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Copy decree binary
COPY --from=builder /build/target/release/decree /usr/local/bin/decree

# Create entrypoint script
RUN cat <<'ENTRYPOINT_EOF' > /usr/local/bin/entrypoint.sh
#!/usr/bin/env bash
set -euo pipefail

# Install AI tool if requested
DECREE_AI="${DECREE_AI:-}"
if [[ -n "$DECREE_AI" ]]; then
  case "$DECREE_AI" in
    opencode)
      if ! command -v opencode &>/dev/null; then
        echo "Installing opencode-ai..."
        npm i -g opencode-ai
      fi
      ;;
    claude)
      if ! command -v claude &>/dev/null; then
        echo "Installing claude-code..."
        npm i -g @anthropic-ai/claude-code
      fi
      ;;
    copilot)
      if ! command -v copilot &>/dev/null; then
        echo "Installing GitHub Copilot CLI..."
        npm i -g @github/copilot
      fi
      ;;
    *)
      echo "ERROR: Unknown DECREE_AI value: $DECREE_AI (supported: opencode, claude, copilot)" >&2
      exit 1
      ;;
  esac
fi

# Initialize decree if .decree/ doesn't exist; without DECREE_AI, init picks the AI on PATH
if [[ ! -d /work/.decree ]]; then
  decree init --no-color ${DECREE_AI:+--ai "$DECREE_AI"}
fi

# If CMD arguments were passed, exec them directly
if [[ $# -gt 0 ]]; then
  exec "$@"
fi

# Default behavior: daemon or interactive shell
DECREE_DAEMON="${DECREE_DAEMON:-true}"
if [[ "$DECREE_DAEMON" == "true" ]]; then
  exec decree daemon --no-color --interval "${DECREE_INTERVAL:-2}"
else
  exec bash
fi
ENTRYPOINT_EOF
RUN chmod +x /usr/local/bin/entrypoint.sh

WORKDIR /work

VOLUME ["/work"]

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
