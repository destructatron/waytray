#!/bin/bash
# Simple uptime script - demonstrates line-based output format
# Mode: interval (run periodically)
#
# Output format (line-based):
#   Line 1: Label text
#   Line 2: Tooltip text (optional)

uptime_str=$(uptime -p | sed 's/^up //')
load=$(cut -d' ' -f1 /proc/loadavg)

echo "$uptime_str"
echo "Load average: $load"
