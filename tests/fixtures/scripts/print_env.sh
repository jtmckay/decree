#!/usr/bin/env bash
# Prints DECREE and every DECREE_ variable, sorted.
env | grep -E "^DECREE(_|=)" | sort
