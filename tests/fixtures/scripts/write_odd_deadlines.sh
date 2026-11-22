#!/usr/bin/env bash
# Deadlines timeout_at must refuse: `link`, a symlink out of the run's folder, and `long`,
# 200 characters that are no instant.
ln -sf /etc/passwd "$DECREE_RUN_DIR/link"
printf '%0200d' 7 > "$DECREE_RUN_DIR/long"
