#!/usr/bin/env bash
# Copies processed.md as it is while this script runs into ledger.txt in the project root.
cp "$DECREE_PROJECT_ROOT/.decree/processed.md" "$DECREE_PROJECT_ROOT/ledger.txt"
