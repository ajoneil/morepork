// gbtrace-mame: a gbtrace adapter for MAME's Atari 2600 driver (VCS family),
// a third independent-lineage behavioural oracle.
//
// MAME is not linkable like the Stella/Gopher2600 adapters, so this drives it
// headlessly via its gdbstub debugger (MAME's gdbstub supports the m6502). For
// speed it does NOT single-step over the wire (that was ~19s/ROM); instead it
// uses the GDB remote `monitor` command (qRcmd) to install MAME's own debugger
// `trace` command (which logs every instruction at full emulation speed) plus a
// watchpoint on the RESULT byte, then `continue`s to the verdict (~250ms/ROM).
// The trace log is parsed into a native .gbtrace via the FFI (no JSONL).
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
	"strings"
	"syscall"
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
	for {
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
	g.r.ReadByte()
	g.r.ReadByte()
	g.conn.Write([]byte("+"))
	return string(body)
}

func (g *gdb) cmd(body string) string { g.send(body); return g.recv() }

// mon runs a MAME debugger console command via the GDB `monitor` (qRcmd) escape.
func (g *gdb) mon(command string) string {
	resp := g.cmd("qRcmd," + hex.EncodeToString([]byte(command)))
	dec, _ := hex.DecodeString(resp)
	return string(dec)
}

// switchLuaTemplate sets the a2600 console switches (best-effort; see README).
const switchLuaTemplate = `
local v = %d
local function apply()
  local swb = manager.machine.ioport.ports[":SWB"]
  if not swb then return end
  local function set(name, bit)
    local f = swb.fields[name]
    if f then f:set_value(((v >> bit) & 1) ~= 0 and f.mask or 0) end
  end
  set("TV Type", 3)
  set("Left Diff. Switch", 6)
  set("Right Diff. Switch", 7)
end
apply()
emu.register_prestart(apply)
emu.register_frame_done(apply)
`

func main() {
	rom := flag.String("rom", "", "path to the .bin/.a26 ROM")
	out := flag.String("out", "trace.gbtrace", "output .gbtrace path")
	spec := flag.String("spec", "NTSC", "TV spec: NTSC or PAL (a2600 vs a2600p)")
	maxFrames := flag.Int("frames", 30, "cap: seconds_to_run = max(2, frames/60)")
	port := flag.Int("port", 23946, "gdbstub port")
	swchb := flag.Int("swchb", 0x48, "console switches: bit3=colour, bit6=P0 diff-A, bit7=P1 diff-A")
	frame := flag.Bool("frame", true, "capture a final frame snapshot (a second headless MAME pass)")
	flag.Parse()
	if *rom == "" {
		fmt.Fprintln(os.Stderr, "error: -rom is required")
		os.Exit(2)
	}
	if err := run(*rom, *out, *spec, *maxFrames, *port, *swchb, *frame); err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}

func run(romPath, outPath, spec string, maxFrames, port, swchb int, wantFrame bool) error {
	romBytes, err := os.ReadFile(romPath)
	if err != nil {
		return err
	}
	romSha := hex.EncodeToString(func() []byte { s := sha256.Sum256(romBytes); return s[:] }())

	luaFile, err := os.CreateTemp("", "gbtrace-mame-*.lua")
	if err != nil {
		return err
	}
	defer os.Remove(luaFile.Name())
	luaFile.WriteString(fmt.Sprintf(switchLuaTemplate, swchb))
	luaFile.Close()
	cfgDir, err := os.MkdirTemp("", "gbtrace-mame-cfg-*")
	if err != nil {
		return err
	}
	defer os.RemoveAll(cfgDir)
	traceLog, err := os.CreateTemp("", "gbtrace-mame-trace-*.log")
	if err != nil {
		return err
	}
	traceLog.Close()
	defer os.Remove(traceLog.Name())

	machine := "a2600"
	if spec == "PAL" {
		machine = "a2600p"
	}
	seconds := maxFrames / 60
	if seconds < 2 {
		seconds = 2
	}
	mame := exec.Command("mame", machine, "-cart", romPath,
		"-video", "none", "-sound", "none", "-nothrottle",
		"-autoboot_script", luaFile.Name(), "-autoboot_delay", "0",
		"-cfg_directory", cfgDir,
		"-debug", "-debugger", "gdbstub", "-debugger_port", strconv.Itoa(port),
		"-seconds_to_run", strconv.Itoa(seconds))
	mame.Stdout, mame.Stderr = nil, nil
	// Own process group so we can reap MAME *and* any child it forks. Killing
	// only the launcher's PID left the real emulator process lingering.
	mame.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := mame.Start(); err != nil {
		return fmt.Errorf("launch mame: %w", err)
	}
	defer func() {
		// SIGKILL the whole group (negative PID), then Wait to reap the zombie.
		// When MAME is paused at the watchpoint it never reaches seconds_to_run,
		// so it will not exit on its own — this is the only thing that stops it.
		if mame.Process != nil {
			_ = syscall.Kill(-mame.Process.Pid, syscall.SIGKILL)
			_ = mame.Process.Kill()
			_, _ = mame.Process.Wait()
		}
	}()

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
	// handshake (MAME's gdbstub answers `g`/monitor only after these)
	g.cmd("qSupported")
	g.cmd("qXfer:features:read:target.xml:0,3fc")

	// install a full-speed per-instruction trace (noloop = don't collapse loops;
	// register symbol is `sp` not `s`) and a watchpoint on the RESULT verdict.
	g.mon(fmt.Sprintf(`trace %s,maincpu,noloop,{tracelog "R%%04X %%02X %%02X %%02X %%02X %%02X\n",pc,a,x,y,sp,p}`, traceLog.Name()))
	g.mon("wpset 0x80,1,w,{(wpdata==0xa5)||(wpdata==0x5a)}")
	conn.SetReadDeadline(time.Now().Add(30 * time.Second))
	g.cmd("c") // run full-speed to the verdict (or seconds_to_run)
	// read the RESULT bytes at the stop (per-instruction memory in the trace
	// format breaks tracelog, so we grab the final verdict here).
	res, code, obs, exp := parseMem(g.cmd("m80,4"))
	g.mon("trace off") // flush the trace file
	_ = syscall.Kill(-mame.Process.Pid, syscall.SIGKILL) // free the port before pass 2

	// A second, gdbstub-free headless pass captures the final frame via Lua
	// (gdbstub exposes no pixels). Best-effort: a frame is nice-to-have.
	var fr *frameData
	if wantFrame {
		if f, ferr := captureFrame(romPath, spec, maxFrames); ferr == nil {
			fr = f
		} else {
			fmt.Fprintf(os.Stderr, "warning: frame capture failed: %v\n", ferr)
		}
	}

	return writeTrace(outPath, spec, romSha, traceLog.Name(), res, code, obs, exp, fr)
}

type frameData struct {
	width, height int
	pixels        []byte // TIA colour codes (canonical-palette indices)
}

// frameLuaTemplate runs the ROM for a few frames, then dumps the screen's
// pixels (ARGB32) to a file and exits. %q = dump path, %d = frame to capture on.
const frameLuaTemplate = `
local target = %d
local n = 0
emu.register_frame_done(function()
  n = n + 1
  if n < target then return end
  local s = manager.machine.screens:at(1)
  local ok, px = pcall(function() return s:pixels() end)
  if ok then
    local f = io.open(%q, "wb"); f:write(px); f:close()
    io.stderr:write(string.format("GBFRAME %%d %%d\n", s.width, s.height))
  end
  manager.machine:exit()
end)
`

// captureFrame launches a second headless MAME, dumps the last frame's pixels,
// and reverse-maps each RGB to a TIA colour code (nearest canonical palette
// entry) so the frame is oracle-independent like the other adapters'.
func captureFrame(romPath, spec string, maxFrames int) (*frameData, error) {
	machine := "a2600"
	if spec == "PAL" {
		machine = "a2600p"
	}
	dump, err := os.CreateTemp("", "gbtrace-mame-px-*.bin")
	if err != nil {
		return nil, err
	}
	dump.Close()
	defer os.Remove(dump.Name())
	luaFile, err := os.CreateTemp("", "gbtrace-mame-frame-*.lua")
	if err != nil {
		return nil, err
	}
	defer os.Remove(luaFile.Name())
	target := maxFrames
	if target < 8 {
		target = 8 // let a static image settle
	}
	luaFile.WriteString(fmt.Sprintf(frameLuaTemplate, target, dump.Name()))
	luaFile.Close()

	seconds := target/60 + 2
	cmd := exec.Command("mame", machine, "-cart", romPath,
		"-video", "none", "-sound", "none", "-nothrottle",
		"-autoboot_script", luaFile.Name(), "-autoboot_delay", "0",
		"-seconds_to_run", strconv.Itoa(seconds))
	var stderr strings.Builder
	cmd.Stderr = &stderr
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()
	select {
	case <-done:
	case <-time.After(30 * time.Second):
	}
	if cmd.Process != nil {
		_ = syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
		_, _ = cmd.Process.Wait()
	}

	// parse "GBFRAME <w> <h>" from stderr
	var w, h int
	for _, ln := range strings.Split(stderr.String(), "\n") {
		if strings.HasPrefix(ln, "GBFRAME ") {
			fmt.Sscanf(ln, "GBFRAME %d %d", &w, &h)
		}
	}
	if w == 0 || h == 0 {
		return nil, fmt.Errorf("no frame dumped (%s)", firstLine(stderr.String()))
	}
	argb, err := os.ReadFile(dump.Name())
	if err != nil {
		return nil, err
	}
	if len(argb) < w*h*4 {
		return nil, fmt.Errorf("short pixel dump: %d < %d", len(argb), w*h*4)
	}
	// MAME's a2600 screen is 176 wide = the 160 visible pixels + an 8px border
	// each side. Centre-crop to the canonical 160-wide visible so the frame
	// matches the other adapters. (Vertical alignment vs the full-field golden
	// is done in the GOLD compare step — MAME exposes no VSYNC position.)
	const vis = 160
	x0 := (w - vis) / 2
	if x0 < 0 {
		x0 = 0
	}
	cw := vis
	if cw > w {
		cw = w
	}
	pixels := make([]byte, cw*h)
	cache := map[uint32]uint8{}
	for y := 0; y < h; y++ {
		for cx := 0; cx < cw; cx++ {
			i := y*w + (x0 + cx)
			// MAME pixels() is ARGB32 little-endian: bytes b,g,r,a.
			b, gg, r := argb[i*4], argb[i*4+1], argb[i*4+2]
			key := uint32(r)<<16 | uint32(gg)<<8 | uint32(b)
			idx, ok := cache[key]
			if !ok {
				idx = nearestCanonical(r, gg, b)
				cache[key] = idx
			}
			pixels[y*cw+cx] = idx
		}
	}
	return &frameData{width: cw, height: h, pixels: pixels}, nil
}

func firstLine(s string) string {
	if i := strings.IndexByte(s, '\n'); i >= 0 {
		return s[:i]
	}
	return s
}

// nearestCanonical maps an RGB triple to the TIA colour code whose canonical
// palette entry is closest (squared distance). Only even indices are real
// colours (odd = black); an exact match is the common case for solid fills.
func nearestCanonical(r, g, b uint8) uint8 {
	best := uint8(0)
	bestD := int32(1<<31 - 1)
	for i := 0; i < 256; i += 2 {
		dr := int32(r) - int32(canonicalNTSCPalette[i*3])
		dg := int32(g) - int32(canonicalNTSCPalette[i*3+1])
		db := int32(b) - int32(canonicalNTSCPalette[i*3+2])
		d := dr*dr + dg*dg + db*db
		if d < bestD {
			bestD = d
			best = uint8(i)
			if d == 0 {
				break
			}
		}
	}
	return best
}

func parseMem(h string) (a, b, c, d uint8) {
	bb := func(i int) uint8 {
		if len(h) < i+2 {
			return 0
		}
		v, _ := strconv.ParseUint(h[i:i+2], 16, 8)
		return uint8(v)
	}
	return bb(0), bb(2), bb(4), bb(6)
}

// writeTrace parses the MAME trace log (R<pc> <a> <x> <y> <sp> <p> lines) into a
// native .gbtrace. The RESULT bytes are placed on the final (verdict) entry.
func writeTrace(outPath, spec, romSha, logPath string, res, code, obs, exp uint8, fr *frameData) error {
	lf, err := os.Open(logPath)
	if err != nil {
		return err
	}
	defer lf.Close()

	type entry struct{ pc uint16; a, x, y, s, p uint8 }
	var entries []entry
	sc := bufio.NewScanner(lf)
	sc.Buffer(make([]byte, 1<<20), 1<<20)
	for sc.Scan() {
		line := sc.Text()
		if len(line) == 0 || line[0] != 'R' {
			continue
		}
		f := strings.Fields(line)
		if len(f) != 6 {
			continue
		}
		hx := func(s string) uint64 { v, _ := strconv.ParseUint(s, 16, 32); return v }
		entries = append(entries, entry{
			pc: uint16(hx(f[0][1:])),
			a:  uint8(hx(f[1])), x: uint8(hx(f[2])), y: uint8(hx(f[3])),
			s: uint8(hx(f[4])), p: uint8(hx(f[5])),
		})
	}
	if len(entries) == 0 {
		return fmt.Errorf("no trace entries (empty MAME trace log)")
	}

	fields := []string{"pc", "a", "x", "y", "s", "p", "result", "code", "observed", "expected"}
	header := map[string]any{
		"_header": true, "format_version": "0.1.0",
		"emulator": "mame", "emulator_version": "adapter", "rom_sha256": romSha,
		"family": "vcs", "model": spec, "profile": "tier1",
		"fields": fields, "trigger": "instruction",
	}
	if fr != nil {
		header["pix_format"] = "indexed8"
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

	for i, e := range entries {
		setU16("pc", e.pc)
		setU8("a", e.a)
		setU8("x", e.x)
		setU8("y", e.y)
		setU8("s", e.s)
		setU8("p", e.p)
		if i == len(entries)-1 { // RESULT verdict lands on the last entry
			setU8("result", res)
			setU8("code", code)
			setU8("observed", obs)
			setU8("expected", exp)
		} else {
			setU8("result", 0)
			setU8("code", 0)
			setU8("observed", 0)
			setU8("expected", 0)
		}
		C.gbtrace_writer_finish_entry(w)
	}
	if fr != nil && fr.width > 0 && fr.height > 0 && len(fr.pixels) > 0 {
		pal := canonicalNTSCPalette
		C.gbtrace_writer_mark_frame_indexed(w,
			C.uint16_t(fr.width), C.uint16_t(fr.height), C.float(12.0/7.0),
			(*C.uint8_t)(unsafe.Pointer(&pal[0])), C.size_t(256),
			(*C.uint8_t)(unsafe.Pointer(&fr.pixels[0])), C.size_t(len(fr.pixels)))
	}
	if C.gbtrace_writer_close(w) != 0 {
		return fmt.Errorf("writer close failed")
	}
	return nil
}
