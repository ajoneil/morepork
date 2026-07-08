; calib_colors — MAME palette calibration ROM (not a test).
;
; MAME's a2600 palette RGB differs from the suite's canonical NTSC palette, so
; mapping MAME's rendered pixels back to TIA colour codes by nearest-canonical
; is imprecise (adjacent luma/hue steps get confused). This ROM displays every
; TIA colour code as a solid full-width scanline so gen_mame_palette.py can read
; MAME's actual RGB for each code and build an exact reverse map.
;
; Visible line L (0-based, from the first active line) shows COLUBK = (L+1)*2,
; i.e. codes 2,4,...,254 on lines 0..126 (code 0 = black lands last). Starting
; at a non-black code lets the generator find the first active line by content.

        processor 6502
        include "vcs.h"
        include "macro.h"

        org $F000

Reset:
        CLEAN_START

MainLoop:
        lda #$02                ; VSYNC (3 lines)
        sta VSYNC
        sta WSYNC
        sta WSYNC
        sta WSYNC
        lda #$00
        sta VSYNC

        lda #$02                ; VBLANK (37 lines)
        sta VBLANK
        ldx #37
.vblank:
        sta WSYNC
        dex
        bne .vblank
        lda #$00
        sta VBLANK

        ldx #$02                ; visible: COLUBK ramps by 2 each line
        ldy #192
.visible:
        stx COLUBK
        sta WSYNC
        inx
        inx
        dey
        bne .visible

        lda #$02                ; overscan (30 lines)
        sta VBLANK
        ldx #30
.overscan:
        sta WSYNC
        dex
        bne .overscan
        lda #$00
        sta VBLANK

        jmp MainLoop

        org $FFFC
        .word Reset
        .word Reset
