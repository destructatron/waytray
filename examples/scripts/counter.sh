#!/bin/bash
# Counter script - demonstrates watch mode (long-running process)
# Mode: watch (daemon monitors stdout, each line triggers an update)
#
# This script runs continuously and outputs a new line periodically.
# Each line should be a complete output (JSON or line-based).

count=0

while true; do
    echo "{\"label\": \"Count: $count\", \"tooltip\": \"Watch mode counter example\", \"icon\": \"accessories-calculator\"}"
    ((count++))
    sleep 5
done
