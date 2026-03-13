# Build stage: compile decree
FROM rust:1-slim AS builder

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
        bash git curl ca-certificates util-linux && \
    rm -rf /var/lib/apt/lists/*

# Install yq
RUN ARCH=$(dpkg --print-architecture) && \
    curl -fsSL "https://github.com/mikefarah/yq/releases/latest/download/yq_linux_${ARCH}" \
        -o /usr/local/bin/yq && \
    chmod +x /usr/local/bin/yq

# Install opencode
RUN npm i -g opencode-ai

# Copy decree binary
COPY --from=builder /build/target/release/decree /usr/local/bin/decree

# Copy entrypoint and sync scripts
COPY docker/entrypoint.sh docker/routine-sync.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/entrypoint.sh /usr/local/bin/routine-sync.sh

WORKDIR /work

VOLUME ["/work", "/routines", "/root/.config/opencode"]

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
