#!/usr/bin/env bash
# Fail fast if the environment every .decree/scripts/*/requirements declares is not all
# there, such as claude or git missing from PATH. An infra message adds what is missing;
# develop does not.
set -euo pipefail
.decree/scripts/infra/run.sh check
