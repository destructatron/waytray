#!/bin/bash
# TODO count script - demonstrates JSON output with scroll actions
# Mode: interval or once
#
# Counts TODO/FIXME comments in a directory (defaults to current dir)
# Scroll up/down to cycle through TODO files

DIR="${1:-.}"

# Count TODOs
todo_count=$(grep -r --include="*.rs" --include="*.py" --include="*.js" --include="*.ts" -c -E "(TODO|FIXME)" "$DIR" 2>/dev/null | awk -F: '{sum += $2} END {print sum+0}')

if [ "$todo_count" -eq 0 ]; then
    label="No TODOs"
    icon="emblem-ok-symbolic"
else
    label="$todo_count TODOs"
    icon="emblem-important-symbolic"
fi

# Get first few files with TODOs for tooltip
files=$(grep -r --include="*.rs" --include="*.py" --include="*.js" --include="*.ts" -l -E "(TODO|FIXME)" "$DIR" 2>/dev/null | head -5 | tr '\n' ', ' | sed 's/,$//')

if [ -n "$files" ]; then
    tooltip="Files: $files"
else
    tooltip="No TODO/FIXME comments found"
fi

cat << EOF
{
  "label": "$label",
  "tooltip": "$tooltip",
  "icon": "$icon"
}
EOF
