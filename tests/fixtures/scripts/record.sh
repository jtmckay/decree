#!/usr/bin/env bash
# Appends its own name to order.txt in the project root; a name ending in _fail then exits 1.
name=$(basename "$0" .sh)
echo "$name" >> "$DECREE_PROJECT_ROOT/order.txt"
case "$name" in *_fail) exit 1 ;; esac
exit 0
