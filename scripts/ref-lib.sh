# Model-aware screenshot reference lookup (sourced by the trace scripts).
#
# Many GBC/DMG-shared screenshot tests ship a *separate* CGB reference that
# captures the CGB's intended palette (e.g. mealybug `_cgb_c`, dmg-acid2
# `-cgb`, blargg `-cgb`, gambatte `_cgb04c`). When running under CGB we must
# compare against that, not the DMG greyscale reference. Tests that render
# identically on both models ship only a DMG reference, so CGB falls back to
# it.
#
# find_ref <rom> <model> echoes the best matching .rgb555 reference path, or
# nothing if none exists. References live next to the ROM.
find_ref() {
    local rom="$1" model="$2" dir parent stem c d
    dir="$(dirname "$rom")"
    parent="$(dirname "$dir")"
    stem="$(basename "$rom")"; stem="${stem%.gbc}"; stem="${stem%.gb}"
    local cands
    if [[ "$model" == cgb ]]; then
        # CGB-specific references first, then fall back to the DMG/base ref.
        cands=("${stem}_cgb04c" "${stem}_cgb_c" "${stem}-cgb" \
               "${stem}" "${stem}-dmg" "${stem}_dmg08")
    else
        cands=("${stem}_dmg08" "${stem}-dmg" "${stem}")
    fi
    # Look next to the ROM and one level up — some suites (e.g. blargg) keep
    # references in the parent dir while ROMs are nested in individual/rom_singles/.
    for c in "${cands[@]}"; do
        for d in "$dir" "$parent"; do
            if [[ -f "$d/$c.rgb555" ]]; then
                echo "$d/$c.rgb555"
                return
            fi
        done
    done
}
