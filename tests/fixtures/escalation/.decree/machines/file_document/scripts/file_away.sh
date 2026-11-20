#!/usr/bin/env bash
# Move the scan to the folder its state names: file_invoice -> filed/invoice/.
set -euo pipefail
dest="filed/${DECREE_STATE#file_}"
mkdir -p "$dest"
git mv "$DECREE_DATA_FILE" "$dest/"
echo "$DECREE_DATA_FILE -> $dest/"
