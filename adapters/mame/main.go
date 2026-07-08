// gbtrace-mame: a gbtrace adapter for MAME's Atari 2600 driver (VCS family),
// a third independent-lineage behavioural oracle.
//
// MAME is not linkable like the Stella/Gopher2600 adapters, so this drives it
// headlessly via its gdbstub debugger (MAME's gdbstub supports the m6502): it
// launches `mame a2600 ... -debug -debugger gdbstub`, speaks the GDB remote
// protocol to step one instruction at a time and read the 6507 register file +
// RAM, and writes a native .gbtrace through the FFI (no JSONL).
//
// Frame snapshots are not available over gdbstub (the GDB protocol has no screen);
// those would come from a parallel Lua screen capture when GOLD tests need them.
//
//	gbtrace-mame -rom test.bin -out trace.gbtrace -spec NTSC -frames 30
package main

/*
#include <stdlib.h>
#include "gbtrace.h"
*/
import "C"

import (
	"bufio"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"net"
	"os"
	"os/exec"
	"strconv"
	"time"
	"unsafe"
)

// --- GDB remote client ---
type gdb struct {
	conn net.Conn
	r    *bufio.Reader
}

func (g *gdb) send(body string) {
	sum := 0
	for i := 0; i < len(body); i++ {
		sum += int(body[i])
	}
	fmt.Fprintf(g.conn, "$%s#%02x", body, sum&0xff)
}

func (g *gdb) recv() string {
	for { // skip acks/noise until packet start
		b, err := g.r.ReadByte()
		if err != nil {
			return ""
		}
		if b == '$' {
			break
		}
	}
	body := make([]byte, 0, 64)
	for {
		b, err := g.r.ReadByte()
		if err != nil {
			return ""
		}
		if b == '#' {
			break
		}
		body = append(body, b)
	}
	g.r.ReadByte() // checksum hi
	g.r.ReadByte() // checksum lo
	g.conn.Write([]byte("+"))
	return string(body)
}

func (g *gdb) cmd(body string) string { g.send(body); return g.recv() }

// parseRegs decodes the `g` response: a x y p (1 byte each) then sp, pc (2 bytes
// little-endian each). Returns a,x,y,p,s(low byte of sp),pc.
func parseRegs(h string) (a, x, y, p, s uint8, pc uint16, ok bool) {
	if len(h) < 16 {
		return
	}
	b := func(i int) uint8 { v, _ := strconv.ParseUint(h[i:i+2], 16, 8); return uint8(v) }
	a, x, y, p = b(0), b(2), b(4), b(6)
	s = b(8)                             // sp little-endian: low byte first
	pc = uint16(b(12)) | uint16(b(14))<<8 // pc little-endian
	ok = true
	return
}

func main() {
	rom := flag.String("rom", "", "path to the .bin/.a26 ROM")
	out := flag.String("out", "trace.gbtrace", "output .gbtrace path")
	spec := flag.String("spec", "NTSC", "TV spec: NTSC or PAL (a2600 vs a2600p)")
	maxFrames := flag.Int("frames", 30, "cap: ~instructions = frames*30000")
	port := flag.Int("port", 23946, "gdbstub port")
	swchb := flag.Int("swchb", 0x48, "console switches: bit3=colour, bit6=P0 diff-A, bit7=P1 diff-A")
	flag.Parse()
	if *rom == "" {
		fmt.Fprintln(os.Stderr, "error: -rom is required")
		os.Exit(2)
	}
	if err := run(*rom, *out, *spec, *maxFrames, *port, *swchb); err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}

// switchLua sets the a2600 console panel switches (colour + P0/P1 difficulty)
// from $MAME_SWCHB so SWCHB reads are deterministic, matching the other adapters.
// switchLuaTemplate is formatted with the SWCHB value (MAME's Lua sandbox hides
// custom env vars, so the value is baked into the script).
const switchLuaTemplate = `
local v = %d
local function apply()
  local swb = manager.machine.ioport.ports[":SWB"]
  if not swb then return end
  -- a set digital field reports its mask; clear reports 0
  local function set(name, bit)
    local f = swb.fields[name]
    if f then f:set_value(((v >> bit) & 1) ~= 0 and f.mask or 0) end
  end
  set("TV Type", 3)
  set("Left Diff. Switch", 6)
  set("Right Diff. Switch", 7)
end
apply()
emu.register_prestart(apply)    -- before each frame's CPU run (the SWCHB read is in frame 1)
emu.register_frame_done(apply)  -- MAME re-polls inputs each frame; hold the switches
`

func run(romPath, outPath, spec string, maxFrames, port, swchb int) error {
	romBytes, err := os.ReadFile(romPath)
	if err != nil {
		return err
	}
	sum := sha256.Sum256(romBytes)
	romSha := hex.EncodeToString(sum[:])

	// write the switch-setting Lua autoboot script to a temp file
	luaFile, err := os.CreateTemp("", "gbtrace-mame-*.lua")
	if err != nil {
		return err
	}
	defer os.Remove(luaFile.Name())
	luaFile.WriteString(fmt.Sprintf(switchLuaTemplate, swchb))
	luaFile.Close()

	// MAME persists switch/DIP state in its machine .cfg; use a throwaway cfg
	// dir so each run starts from defaults and the Lua switch settings are
	// authoritative (otherwise the first run's switches stick).
	cfgDir, err := os.MkdirTemp("", "gbtrace-mame-cfg-*")
	if err != nil {
		return err
	}
	defer os.RemoveAll(cfgDir)

	machine := "a2600"
	if spec == "PAL" {
		machine = "a2600p"
	}
	// launch MAME headless with the gdbstub debugger + switch-setting script
	mame := exec.Command("mame", machine, "-cart", romPath,
		"-video", "none", "-sound", "none", "-nothrottle",
		"-autoboot_script", luaFile.Name(), "-autoboot_delay", "0",
		"-cfg_directory", cfgDir,
		"-debug", "-debugger", "gdbstub", "-debugger_port", strconv.Itoa(port))
	mame.Stdout, mame.Stderr = nil, nil
	if err := mame.Start(); err != nil {
		return fmt.Errorf("launch mame: %w", err)
	}
	defer func() { _ = mame.Process.Kill() }()

	// wait for the gdbstub to listen
	var conn net.Conn
	for i := 0; i < 100; i++ {
		conn, err = net.Dial("tcp", fmt.Sprintf("localhost:%d", port))
		if err == nil {
			break
		}
		time.Sleep(50 * time.Millisecond)
	}
	if conn == nil {
		return fmt.Errorf("gdbstub never listened on %d", port)
	}
	defer conn.Close()
	g := &gdb{conn: conn, r: bufio.NewReader(conn)}
	conn.Write([]byte("+"))
	// Handshake: MAME's gdbstub only answers `g` (read registers) after the
	// client negotiates features and fetches the target description.
	g.cmd("qSupported")
	g.cmd("qXfer:features:read:target.xml:0,3fc")
	g.cmd("?") // stop reason (machine paused at reset)

	// --- gbtrace writer (native, via FFI) ---
	fields := []string{"pc", "a", "x", "y", "s", "p", "result", "code", "observed", "expected"}
	header := map[string]any{
		"_header": true, "format_version": "0.1.0",
		"emulator": "mame", "emulator_version": "adapter-mvp",
		"rom_sha256": romSha, "family": "vcs", "model": spec,
		"profile": "tier1", "fields": fields, "trigger": "instruction",
	}
	hj, _ := json.Marshal(header)
	cPath := C.CString(outPath)
	defer C.free(unsafe.Pointer(cPath))
	cHdr := C.CString(string(hj))
	defer C.free(unsafe.Pointer(cHdr))
	w := C.gbtrace_writer_new(cPath, cHdr, C.size_t(len(hj)))
	if w == nil {
		return fmt.Errorf("gbtrace_writer_new returned NULL")
	}
	col := map[string]C.int{}
	for _, n := range fields {
		cn := C.CString(n)
		col[n] = C.gbtrace_writer_find_field(w, cn)
		C.free(unsafe.Pointer(cn))
	}
	setU8 := func(n string, v uint8) { C.gbtrace_writer_set_u8(w, C.size_t(col[n]), C.uint8_t(v)) }
	setU16 := func(n string, v uint16) { C.gbtrace_writer_set_u16(w, C.size_t(col[n]), C.uint16_t(v)) }

	// --- drive loop: step, read regs + RESULT bytes, one entry per instruction ---
	maxInstr := maxFrames * 30000
	for i := 0; i < maxInstr; i++ {
		g.cmd("s") // step one instruction
		regs := g.cmd("g")
		a, x, y, p, s, pc, ok := parseRegs(regs)
		if !ok {
			break
		}
		mem := g.cmd("m80,4") // RESULT/CODE/OBSERVED/EXPECTED at $80-$83
		var res, code, obs, exp uint8
		if len(mem) >= 8 {
			bb := func(i int) uint8 { v, _ := strconv.ParseUint(mem[i:i+2], 16, 8); return uint8(v) }
			res, code, obs, exp = bb(0), bb(2), bb(4), bb(6)
		}
		setU16("pc", pc)
		setU8("a", a)
		setU8("x", x)
		setU8("y", y)
		setU8("s", s)
		setU8("p", p)
		setU8("result", res)
		setU8("code", code)
		setU8("observed", obs)
		setU8("expected", exp)
		C.gbtrace_writer_finish_entry(w)
		if res == 0xA5 || res == 0x5A {
			break
		}
	}
	if C.gbtrace_writer_close(w) != 0 {
		return fmt.Errorf("writer close failed")
	}
	return nil
}
