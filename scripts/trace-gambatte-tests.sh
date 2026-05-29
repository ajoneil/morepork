#!/usr/bin/env bash
# Generate a single Gambatte test trace: adapter + ROM → .gbtrace
#
# All Gambatte tests run for exactly 15 LCD frames (1,053,360 T-cycles).
# Pass/fail is determined by the test type, encoded in the filename:
#   1. screenshot  — a {stem}_<model>.pix reference exists → adapter reference match
#   2. _blank      — final frame must be entirely background
#   3. _outaudio0/1 — last-frame audio activity (adapter prints AUDIO=0/1)
#   4. _out<HEX>   — final frame's top tile-row decodes to <HEX>
#   5. _xout       — expected failure, skipped
#
# The expected value is read for the active model ($MODEL, default dmg):
#   shared `_dmg08_cgb04c_out<H>` applies to both; dual
#   `_dmg08_out<H1>_cgb04c_out<H2>` gives H1 for dmg, H2 for cgb.
#
# Model is passed via the MODEL env var (dmg|cgb). The adapter binary is
# passed by the caller (gen-rules resolves docboy+cgb → gbtrace-docboy-cgb).
#
# Usage: MODEL=dmg trace-gambatte-tests.sh <adapter-binary> <rom> <profile> <output-dir> <rom-dir>
set -euo pipefail

BIN="$1"
ROM="$2"
PROFILE="$3"
OUT_DIR="$4"
ROM_DIR="${5:-$(dirname "$ROM")}"
MODEL="${MODEL:-dmg}"
source "$(dirname "$0")/ref-lib.sh"
CLI="${CLI:-target/release/gbtrace}"

# Emulator name: strip gbtrace- prefix and any -cgb build suffix.
ADAPTER="$(basename "$BIN" | sed 's/gbtrace-//; s/-cgb$//')"

TEST_TIMEOUT=120
MAX_FRAMES=15
# gambatte's testrunner reads the screen after a fixed T-cycle budget (15 frames
# × 70224), not after N vblank events. Hex/blank tests use this budget so the
# screen is sampled at the right instant even when the display toggles or the
# CPU stalls — vblank counting samples the wrong frame and fails tests every
# emulator (gambatte included) actually passes.
CYCLE_BUDGET=$((MAX_FRAMES * 70224))

# Test name = path relative to ROM_DIR, subdirs flattened with __
ROM_REL="$(realpath --relative-to="$ROM_DIR" "$ROM")"
ROM_REL="${ROM_REL%.gbc}"; ROM_REL="${ROM_REL%.gb}"
NAME="${ROM_REL//\//__}"
STEM="$(basename "$ROM")"; STEM="${STEM%.gbc}"; STEM="${STEM%.gb}"

# Skip expected-failure tests
if [[ "$NAME" == *_xout* ]]; then
    printf "%-50s %-10s %-4s SKIP (xout)\n" "$NAME" "$ADAPTER" "$MODEL"
    exit 0
fi

# Extract the model-appropriate expected hex / audio outcome from the stem.
#   shared `_dmg08_cgb04c_out<X>` wins; otherwise the model-specific marker.
extract_marker() {  # $1=marker suffix (out|outaudio)
    local m="$1" h
    h=$(grep -oP "(?<=_dmg08_cgb04c_${m})[0-9A-Fa-f]+" <<<"$STEM" | head -1) || true
    if [[ -z "$h" ]]; then
        if [[ "$MODEL" == cgb ]]; then
            h=$(grep -oP "(?<=_cgb04c_${m})[0-9A-Fa-f]+" <<<"$STEM" | head -1) || true
        else
            h=$(grep -oP "(?<=_dmg08_${m})[0-9A-Fa-f]+" <<<"$STEM" | head -1) || true
        fi
    fi
    echo "$h"
}

# Model-aware screenshot reference (gambatte uses {stem}_dmg08 / _cgb04c).
PIX_REF="$(find_ref "$ROM" "$MODEL")"

TMP="/tmp/gbtrace_gambatte_${NAME}_${ADAPTER}_${MODEL}_$$"
stderr_file="${TMP}.stderr"
tmp_trace="${TMP}.gbtrace"
cleanup() { rm -f "$stderr_file" "$tmp_trace" "${ROM%.gb}.sav" "${ROM%.gbc}.sav"; rm -rf "${TMP}_render"; }
trap cleanup EXIT

# Classify the test type. The blank marker is the output tag `_blank` (e.g.
# `_dmg08_cgb_blank`) — match that exactly, NOT a bare "blank", which also occurs
# mid-name in "vblank"/"afterVblank" tests (those are hex tests, not blank ones).
if [[ -f "$PIX_REF" ]]; then
    TYPE="screenshot"
elif [[ "$STEM" == *_blank* ]]; then
    TYPE="blank"
elif [[ "$STEM" == *outaudio* ]]; then
    TYPE="audio"
else
    TYPE="hex"
fi

# --- Capture ---
# hex/blank: cycle budget (read the screen at a fixed T-cycle, like gambatte's
# testrunner). screenshot: frame budget + live reference match. audio: frame
# budget + last-frame audio report.
EXTRA_ARGS=(--model "$MODEL")
case "$TYPE" in
    screenshot) EXTRA_ARGS+=(--reference "$PIX_REF" --frames "$MAX_FRAMES") ;;
    audio)      EXTRA_ARGS+=(--report-audio --frames "$MAX_FRAMES") ;;
    blank|hex)  EXTRA_ARGS+=(--until-tcycle "$CYCLE_BUDGET") ;;
esac

(
    set +eo pipefail
    timeout "$TEST_TIMEOUT" "$BIN" --rom "$ROM" --profile "$PROFILE" \
        "${EXTRA_ARGS[@]}" \
        --output "$tmp_trace" >/dev/null 2>"$stderr_file" </dev/null
) || true

if [[ ! -s "$tmp_trace" ]]; then
    err_msg=$(head -1 "$stderr_file" 2>/dev/null || echo "unknown")
    printf "%-50s %-10s %-4s ERROR (%s)\n" "$NAME" "$ADAPTER" "$MODEL" "$err_msg"
    exit 1
fi

# --- Determine pass/fail ---
status="fail"
case "$TYPE" in
    screenshot)
        grep -q "Reference match" "$stderr_file" 2>/dev/null && status="pass"
        ;;
    audio)
        expected=$(extract_marker outaudio)
        got=$(grep -oP '(?<=AUDIO=)[01]' "$stderr_file" 2>/dev/null | tail -1) || true
        [[ -n "$expected" && "$got" == "$expected" ]] && status="pass"
        ;;
    blank|hex)
        tmp_render="${TMP}_render"; mkdir -p "$tmp_render"
        # The cycle-budget run emits the screen-at-budget as the trace's last
        # frame, so render all frames and take the final one.
        timeout 30 "$CLI" render "$tmp_trace" --output "$tmp_render" >/dev/null 2>&1 || true
        png=$(ls "$tmp_render"/*.png 2>/dev/null | sort -V | tail -1)
        if [[ -n "$png" ]]; then
            if [[ "$TYPE" == "blank" ]]; then
                python3 "$(dirname "$0")/check-gambatte-hex.py" --blank "$png" 2>/dev/null && status="pass"
            else
                expected=$(extract_marker out)
                [[ -n "$expected" ]] && python3 "$(dirname "$0")/check-gambatte-hex.py" "$expected" "$png" 2>/dev/null && status="pass"
            fi
        fi
        ;;
esac

# --- Output ---
mkdir -p "$OUT_DIR"
out="${OUT_DIR}/${NAME}_${ADAPTER}_${MODEL}_${status}.gbtrace"
mv "$tmp_trace" "$out"

entries=$("$CLI" info "$out" 2>/dev/null | grep Entries | awk '{print $2}')
printf "%-50s %-10s %-4s %-4s %6s entries\n" "$NAME" "$ADAPTER" "$MODEL" "${status^^}" "${entries:-?}"
