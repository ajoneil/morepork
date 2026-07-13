#!/usr/bin/env bash
# Generate a single blargg trace: adapter + ROM → .morepork
#
# Pass/fail: adapter reports "Reference match" when framebuffer matches .pix
#
# Usage: trace-blargg.sh <adapter-binary> <rom> <profile> <output-dir> <rom-dir> [max-frames]
set -euo pipefail

BIN="$1"
ROM="$2"
PROFILE="$3"
OUT_DIR="$4"
ROM_DIR="${5:-$(dirname "$ROM")}"
CLI="${CLI:-target/release/morepork}"

ADAPTER="$(basename "$BIN" | sed 's/morepork-//; s/-cgb$//')"
MODEL="${MODEL:-dmg}"
source "$(dirname "$0")/ref-lib.sh"

# Use relative path from ROM_DIR as the test name, flattening subdirs with __
ROM_REL="$(realpath --relative-to="$ROM_DIR" "$ROM")"
ROM_REL="${ROM_REL%.gbc}"; ROM_REL="${ROM_REL%.gb}"
NAME="${ROM_REL//\//__}"

# Check for .pix reference next to the ROM
BASENAME="$(basename "$ROM")"; BASENAME="${BASENAME%.gbc}"; BASENAME="${BASENAME%.gb}"
PIX_REF="$(find_ref "$ROM" "$MODEL")"

# Frame budget: optional 6th positional arg (default 2000), mirroring
# trace-screenshot-suite.sh. cgb_sound only reaches its "Passed" screen at
# ~frame 2184, so gen-rules passes a larger budget for that sub-suite.
MAX_FRAMES="${6:-2000}"
TMP="/tmp/morepork_blargg_${NAME}_${ADAPTER}_$$"
stderr_file="${TMP}.stderr"
tmp_trace="${TMP}.morepork"

cleanup() { rm -f "$stderr_file" "$tmp_trace" "${ROM%.gb}.sav" "${ROM%.gbc}.sav"; }
trap cleanup EXIT

# --- Capture ---
EXTRA_ARGS=()
if [[ -f "$PIX_REF" ]]; then
    EXTRA_ARGS+=(--reference "$PIX_REF")
fi

(
    set +eo pipefail
    "$BIN" --rom "$ROM" --profile "$PROFILE" --model "$MODEL" \
        --extra-frames 2 --frames "$MAX_FRAMES" \
        "${EXTRA_ARGS[@]}" \
        --output "$tmp_trace" >/dev/null 2>"$stderr_file" </dev/null
) || true

if [[ ! -s "$tmp_trace" ]]; then
    err_msg=$(head -1 "$stderr_file" 2>/dev/null || echo "unknown")
    printf "%-40s %-10s ERROR (%s)\n" "$NAME" "$ADAPTER" "$err_msg"
    exit 1
fi

# --- Determine pass/fail ---
status="fail"
if grep -q "Reference match" "$stderr_file" 2>/dev/null; then
    status="pass"
fi

# --- Output ---
mkdir -p "$OUT_DIR"
out="${OUT_DIR}/${NAME}_${ADAPTER}_${MODEL}_${status}.morepork"
mv "$tmp_trace" "$out"

entries=$("$CLI" info "$out" 2>/dev/null | grep Entries | awk '{print $2}')
printf "%-40s %-10s %-4s %6s entries\n" "$NAME" "$ADAPTER" "${status^^}" "${entries:-?}"
