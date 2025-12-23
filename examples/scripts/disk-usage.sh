#!/bin/bash
# Disk usage script - demonstrates JSON output with actions
# Mode: interval (run periodically)
#
# Output format (JSON):
#   label: Display text
#   tooltip: Hover text
#   icon: Icon name from theme
#   actions: Array of {id, command} for click/scroll handling

# Get root filesystem usage
usage=$(df -h / | awk 'NR==2 {print $5}' | tr -d '%')
used=$(df -h / | awk 'NR==2 {print $3}')
total=$(df -h / | awk 'NR==2 {print $2}')
avail=$(df -h / | awk 'NR==2 {print $4}')

# Choose icon based on usage
if [ "$usage" -ge 90 ]; then
    icon="drive-harddisk-warning"
elif [ "$usage" -ge 70 ]; then
    icon="drive-harddisk"
else
    icon="drive-harddisk"
fi

# Output JSON
cat << EOF
{
  "label": "Disk ${usage}%",
  "tooltip": "Root filesystem\nUsed: ${used} / ${total}\nAvailable: ${avail}",
  "icon": "${icon}",
  "actions": [
    {"id": "Activate", "command": "xdg-open /"}
  ]
}
EOF
