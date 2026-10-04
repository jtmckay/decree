#!/usr/bin/env bash
# Invoke of to_develop and to_feature. The state name carries the router's
# choice (to_<machine>); `decree emit` refuses any machine not in `emits`.
set -euo pipefail
machine="${DECREE_STATE#to_}"
# The body is everything after the closing frontmatter fence.
awk 'NR==1 && $0=="---" {fm=1; next} fm && $0=="---" {fm=0; next} !fm' "$DECREE_MESSAGE" \
  | decree emit --machine "$machine"
