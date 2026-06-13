#!/usr/bin/env bash
# ============================================================================
# fetch-aerial.sh  —  Google Aerial View API: address -> cinematic MP4 orbit
#
# The MP4 is a smooth fly-around of the address; feed it straight into
# ./pipeline/splat.sh to turn it into a 3D Gaussian Splat.
#
# Usage:
#   GOOGLE_MAPS_API_KEY=xxxx ./pipeline/fetch-aerial.sh "500 W 2nd St, Austin, TX 78701"
#   GOOGLE_MAPS_API_KEY=xxxx ./pipeline/fetch-aerial.sh "Schelvisch Hoofd 32, 1035 JV, Amsterdam" myorbit.mp4
#
# Needs: a Google Cloud project with the *Aerial View API* enabled, and an
# API key (env GOOGLE_MAPS_API_KEY or AERIAL_VIEW_API_KEY). The endpoint
# itself is currently free of charge.
#
# Notes:
#   * Rendering a NEW address takes ~1 hour to a few hours (async). Already-
#     rendered addresses come back instantly (Google caches per address).
#   * Coverage is heavily US-centric. Non-US addresses may return no imagery.
#   * Download URIs are signed + short-lived — grab the MP4 promptly.
# ============================================================================
set -euo pipefail

ADDRESS="${1:?Usage: GOOGLE_MAPS_API_KEY=xxx ./fetch-aerial.sh \"<postal address>\" [out.mp4]}"
OUT="${2:-aerial.mp4}"
KEY="${GOOGLE_MAPS_API_KEY:-${AERIAL_VIEW_API_KEY:-}}"
ORIENT="${ORIENT:-landscape}"          # landscape | portrait
MAX_WAIT="${MAX_WAIT:-7200}"           # seconds to keep polling before giving up
POLL_ONLY="${POLL_ONLY:-0}"            # 1 = treat $1 as a videoId, skip render

[ -n "$KEY" ] || { echo "ERROR: set GOOGLE_MAPS_API_KEY (or AERIAL_VIEW_API_KEY)"; exit 1; }

API="https://aerialview.googleapis.com/v1"
URI_FIELD="${ORIENT}Uri"

# --- 1. kick off (or look up) the render -----------------------------------
if [ "$POLL_ONLY" = "1" ]; then
  VIDEO_ID="$ADDRESS"
  echo "==> Polling existing videoId: $VIDEO_ID"
else
  echo "==> renderVideo  <-  \"$ADDRESS\""
  RESP="$(curl -sS -X POST "$API/videos:renderVideo" \
    -H "Content-Type: application/json" \
    -H "X-Goog-Api-Key: $KEY" \
    -d "$(jq -nc --arg a "$ADDRESS" '{address:$a}')")"

  # API errors (e.g. no coverage / bad address) come back as {"error":{...}}
  if echo "$RESP" | jq -e '.error' >/dev/null 2>&1; then
    echo "    API rejected the request — likely NO AERIAL COVERAGE for this address:"
    echo "$RESP" | jq '.error | {code, status, message}'
    exit 2
  fi

  VIDEO_ID="$(echo "$RESP" | jq -r '.metadata.videoId // .videoId // empty')"
  STATE="$(echo "$RESP" | jq -r '.state // empty')"
  echo "    state=$STATE  videoId=$VIDEO_ID"
  [ -n "$VIDEO_ID" ] || { echo "    no videoId in response:"; echo "$RESP" | jq .; exit 2; }
fi

# --- 2. poll lookupVideo until ACTIVE --------------------------------------
echo "==> Polling lookupVideo (max ${MAX_WAIT}s; new renders can take hours)"
WAITED=0; DELAY=5
while :; do
  L="$(curl -sS "$API/videos:lookupVideo?videoId=${VIDEO_ID}" -H "X-Goog-Api-Key: $KEY")"
  STATE="$(echo "$L" | jq -r '.state // "UNKNOWN"')"
  printf '    [%5ds] state=%s\n' "$WAITED" "$STATE"
  case "$STATE" in
    ACTIVE)  break ;;
    FAILED)  echo "    render FAILED:"; echo "$L" | jq .; exit 3 ;;
    PROCESSING|UNKNOWN) : ;;
    *) echo "    unexpected state; full response:"; echo "$L" | jq . ;;
  esac
  [ "$WAITED" -lt "$MAX_WAIT" ] || { echo "    gave up after ${MAX_WAIT}s. Re-run later with:"; echo "    POLL_ONLY=1 GOOGLE_MAPS_API_KEY=\$KEY $0 $VIDEO_ID $OUT"; exit 4; }
  sleep "$DELAY"; WAITED=$((WAITED+DELAY)); DELAY=$(( DELAY<60 ? DELAY*2 : 60 ))
done

# --- 3. pick the highest-res MP4 and download ------------------------------
echo "==> ACTIVE. Available media formats:"
echo "$L" | jq -r '.uris | keys[]' | sed 's/^/      /'

# Prefer the richest format; fall back to whatever's first.
URL="$(echo "$L" | jq -r --arg f "$URI_FIELD" '
  .uris as $u
  | ([ "MP4_HIGHEST","MP4_HIGH","MP4_MEDIUM","MP4_LOW" ]
     | map(select($u[.])) | .[0]) as $best
  | ($best // ($u|keys[0])) as $k
  | $u[$k][$f] // empty')"

[ -n "$URL" ] || { echo "ERROR: no $URI_FIELD in any format. Raw uris:"; echo "$L" | jq '.uris'; exit 5; }

echo "==> Downloading $ORIENT MP4 -> $OUT"
curl -sS -L -o "$OUT" "$URL"
echo "    $(du -h "$OUT" | cut -f1)  $OUT"
echo
echo "============================================================"
echo "Next:  ./pipeline/splat.sh \"$OUT\""
echo "  (tune: FPS=4 for more frames, then crop floaters at superspl.at)"
echo "============================================================"
