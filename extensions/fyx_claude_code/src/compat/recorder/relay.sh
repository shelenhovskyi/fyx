#!/bin/sh
# Stand-in for fyx-relay: posts the hook's stdin to the recorder and exits 0.
# Usage (from the plugin's hooks): "$FYX_RELAY" --channel <channel>
channel="$2"
curl -s -m 4 --data-binary @- "http://127.0.0.1:${FYX_REC_PORT}/ctl/hook?channel=${channel}" >/dev/null 2>&1
exit 0
