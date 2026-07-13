#!/usr/bin/env bash
# Generate a single mooneye trace: adapter + ROM → .morepork
#
# Pass/fail detection:
#   Mooneye tests execute LD B, B (opcode 0x40) when complete.
#   Pass: B=3 C=5 D=8 E=13 H=21 L=34 (Fibonacci sequence)
#   Fail: all registers = 0x42
#
# Usage: trace-mooneye.sh <adapter-binary> <rom> <profile> <output-dir> <rom-dir>
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

MAX_FRAMES=500
TMP="/tmp/morepork_mooneye_${NAME}_${ADAPTER}_$$"
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
        --stop-opcode 40 --extra-frames 2 --frames "$MAX_FRAMES" \
        "${EXTRA_ARGS[@]}" \
        --output "$tmp_trace" >/dev/null 2>"$stderr_file" </dev/null
) || true

if [[ ! -s "$tmp_trace" ]]; then
    err_msg=$(head -1 "$stderr_file" 2>/dev/null || echo "unknown")
    printf "%-40s %-10s ERROR (%s)\n" "$NAME" "$ADAPTER" "$err_msg"
    exit 1
fi

# --- Determine pass/fail ---
# Most mooneye tests use the register protocol: on completion they run LD B,B
# with the Fibonacci sequence in the registers (pass) or all 0x42 (fail). A few
# are visual instead (e.g. manual-only/sprite_priority) and ship a screenshot
# reference next to the ROM — judge those by whether the framebuffer matched,
# matching how trace-screenshot-suite.sh decides.
if [[ -f "$PIX_REF" ]]; then
    grep -q "Reference match" "$stderr_file" 2>/dev/null && status="pass" || status="fail"
else
    status=$("$CLI" query "$tmp_trace" --last 1 2>&1 | \
        grep -qP 'b=03\b.*c=05\b.*d=08\b.*e=0d\b.*h=15\b.*l=22\b' \
        && echo "pass" || echo "fail")
fi

# --- Output ---
mkdir -p "$OUT_DIR"
out="${OUT_DIR}/${NAME}_${ADAPTER}_${MODEL}_${status}.morepork"
mv "$tmp_trace" "$out"

entries=$("$CLI" info "$out" 2>/dev/null | grep Entries | awk '{print $2}')
printf "%-40s %-10s %-4s %6s entries\n" "$NAME" "$ADAPTER" "${status^^}" "${entries:-?}"
