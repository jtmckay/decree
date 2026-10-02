#!/usr/bin/env bash
# Prints 60 numbered stdout lines, then two empty ones.
for i in $(seq 1 60); do echo "line $i"; done
echo
echo
