// gbtrace-stella: a gbtrace adapter for the Stella emulator (VCS family).
//
// Drives Stella's emucore headlessly one CPU instruction at a time (via the
// libretro OSystem for setup, but the emucore directly for stepping) and writes
// a native .gbtrace: per-instruction 6507 registers, TIA beam position, and the
// test-suite RESULT convention RAM bytes. An independent per-instruction oracle
// alongside the Gopher2600 adapter.
//
//   gbtrace-stella -rom test.bin -out trace.gbtrace -spec NTSC -frames 30

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>
#include <sys/stat.h>

#include "StellaLIBRETRO.hxx"
#include "SettingsLIBRETRO.hxx"
#include "Console.hxx"
#include "System.hxx"
#include "M6502.hxx"
#include "TIA.hxx"
#include "M6532.hxx"
#include "Switches.hxx"
#include "libretro.h"

#include "gbtrace.h"
#include "ntsc_palette.h"   // canonical VCS NTSC palette (see genpalette.py)

// --- minimal libretro glue referenced by the Stella core ---
// We drive the emucore directly (no libretro frontend). FSNodeLIBRETRO needs a
// VFS with a working stat() to recognise the ROM as a file; we provide a real
// filesystem stat, and the base FSNode then reads the .bin from disk normally.
static int vfs_stat(const char* path, int32_t* size) {
  struct stat st;
  if (::stat(path, &st) != 0) return 0;
  if (size) *size = (int32_t)st.st_size;
  int flags = RETRO_VFS_STAT_IS_VALID;
  if (S_ISDIR(st.st_mode)) flags |= RETRO_VFS_STAT_IS_DIRECTORY;
  return flags;
}
static retro_vfs_interface make_vfs() {
  retro_vfs_interface v{};
  v.stat = vfs_stat;
  return v;
}
static retro_vfs_interface g_vfs = make_vfs();
retro_vfs_interface* libretro_vfs = &g_vfs;  // NOLINT
string libretro_rom_path;                     // NOLINT
string libretro_save_dir;                     // NOLINT
static const uint8_t* g_rom = nullptr;
static uInt32 g_romSize = 0;
uInt32 libretro_get_rom_size() { return g_romSize; }
uInt32 libretro_read_rom(void* data) {
  if (!g_rom) return 0;
  std::memcpy(data, g_rom, g_romSize);
  return g_romSize;
}
void libretro_logger(int, const char*) {}
void post_message(const char*, retro_log_level, unsigned) {}
void libretro_show_message(const char*) {}
void update_input() {}

static std::string jsonHeader(const std::string& spec, const std::string& romSha,
                              bool withFrame) {
  // fields: pc a x y s p line clock result code observed expected
  std::string h = "{";
  h += "\"_header\":true,";
  h += "\"format_version\":\"0.1.0\",";
  h += "\"emulator\":\"stella\",";
  h += "\"emulator_version\":\"adapter-mvp\",";
  h += "\"rom_sha256\":\"" + romSha + "\",";
  h += "\"family\":\"vcs\",";
  h += "\"model\":\"" + spec + "\",";
  h += "\"profile\":\"tier1\",";
  h += "\"fields\":[\"pc\",\"a\",\"x\",\"y\",\"s\",\"p\",\"line\",\"clock\","
       "\"result\",\"code\",\"observed\",\"expected\"],";
  if (withFrame) h += "\"pix_format\":\"indexed8\",";
  h += "\"trigger\":\"instruction\"";
  h += "}";
  return h;
}

// tiny hex sha256-less id: we don't have a crypto lib linked, so hash the ROM
// with a simple FNV-1a and hex-encode. (Adapters only need a stable id here.)
static std::string romId(const std::vector<uint8_t>& rom) {
  uint64_t h = 1469598103934665603ULL;
  for (uint8_t b : rom) { h ^= b; h *= 1099511628211ULL; }
  char buf[17];
  std::snprintf(buf, sizeof(buf), "%016llx", (unsigned long long)h);
  return std::string(buf);
}

int main(int argc, char** argv) {
  const char* rom = nullptr;
  const char* out = "trace.gbtrace";
  std::string spec = "NTSC";
  int maxFrames = 30;
  int swchb = 0x48;   // bit3=colour, bit6=P0 diff-A, bit7=P1 diff-A
  bool wantFrame = true;   // embed a final frame snapshot (GOLD); -frame=false to skip
  for (int i = 1; i < argc; i++) {
    std::string a = argv[i];
    auto next = [&]() { return (i + 1 < argc) ? argv[++i] : ""; };
    if (a == "-rom") rom = next();
    else if (a == "-out") out = next();
    else if (a == "-spec") spec = next();
    else if (a == "-frames") maxFrames = std::atoi(next());
    else if (a == "-swchb") swchb = (int)std::strtol(next(), nullptr, 0);
    else if (a == "-frame") wantFrame = true;
    else if (a == "-frame=false" || a == "-frame=0") wantFrame = false;
    else if (a == "-frame=true" || a == "-frame=1") wantFrame = true;
  }
  if (!rom) { std::fprintf(stderr, "error: -rom is required\n"); return 2; }

  FILE* f = std::fopen(rom, "rb");
  if (!f) { std::fprintf(stderr, "error: cannot open %s\n", rom); return 1; }
  std::fseek(f, 0, SEEK_END);
  long sz = std::ftell(f);
  std::fseek(f, 0, SEEK_SET);
  std::vector<uint8_t> data(sz);
  if (std::fread(data.data(), 1, sz, f) != (size_t)sz) { std::fclose(f); return 1; }
  std::fclose(f);

  // --- headless Stella setup ---
  g_rom = data.data();
  g_romSize = (uInt32)data.size();
  libretro_rom_path = rom;
  StellaLIBRETRO stella;
  stella.setROM(rom, data.data(), (uInt32)data.size());
  SettingsLIBRETRO cfg;
  cfg.console_format = spec;   // NTSC / PAL / PAL60 / SECAM / AUTO
  if (!stella.create(cfg, false)) {
    std::fprintf(stderr, "error: Stella create() failed\n");
    return 1;
  }
  Console& console = stella.osystem().console();

  // Set the console panel switches to a known state (latching colour and
  // difficulty switches) so SWCHB reads are deterministic.
  Switches& sw = console.switches();
  sw.setTvColor((swchb & 0x08) != 0);
  sw.setLeftDifficultyA((swchb & 0x40) != 0);
  sw.setRightDifficultyA((swchb & 0x80) != 0);

  System& system = console.system();
  M6502& cpu = system.m6502();
  TIA& tia = console.tia();
  const uInt8* ram = system.m6532().getRAM().data();

  // --- gbtrace writer ---
  std::string header = jsonHeader(spec, romId(data), wantFrame);
  GbtraceWriter* w = gbtrace_writer_new(out, header.c_str(), header.size());
  if (!w) { std::fprintf(stderr, "error: gbtrace_writer_new failed\n"); return 1; }

  auto col = [&](const char* n) {
    int c = gbtrace_writer_find_field(w, n);
    if (c < 0) { std::fprintf(stderr, "error: field %s missing\n", n); std::exit(1); }
    return (size_t)c;
  };
  size_t cPC = col("pc"), cA = col("a"), cX = col("x"), cY = col("y"),
         cS = col("s"), cP = col("p"), cLine = col("line"), cClk = col("clock"),
         cRes = col("result"), cCode = col("code"), cObs = col("observed"),
         cExp = col("expected");

  // frameCount() is behind DEBUGGER_SUPPORT (absent in the libretro build), so
  // bound the run by an instruction budget (~30k instr/frame upper bound). The
  // real stop is the RESULT verdict.
  const long instrBudget = (long)maxFrames * 30000;
  long instrCount = 0;
  for (;;) {
    cpu.execute(1);              // step one instruction (advances TIA/RIOT)

    gbtrace_writer_set_u16(w, cPC, cpu.gbPC());
    gbtrace_writer_set_u8(w, cA, cpu.gbA());
    gbtrace_writer_set_u8(w, cX, cpu.gbX());
    gbtrace_writer_set_u8(w, cY, cpu.gbY());
    gbtrace_writer_set_u8(w, cS, cpu.gbSP());
    gbtrace_writer_set_u8(w, cP, cpu.gbPS());
    gbtrace_writer_set_u16(w, cLine, (uint16_t)tia.scanlines());
    gbtrace_writer_set_u8(w, cClk, (uint8_t)tia.clocksThisLine());
    gbtrace_writer_set_u8(w, cRes, ram[0x00]);  // $80 RESULT
    gbtrace_writer_set_u8(w, cCode, ram[0x01]); // $81 CODE
    gbtrace_writer_set_u8(w, cObs, ram[0x02]);  // $82 OBSERVED
    gbtrace_writer_set_u8(w, cExp, ram[0x03]);  // $83 EXPECTED
    gbtrace_writer_finish_entry(w);

    if (++instrCount >= instrBudget) break;
    uInt8 r = ram[0x00];
    if (r == 0xA5 || r == 0x5A) break;  // terminal verdict
  }

  // --- final frame snapshot (GOLD modality) ---
  // Stella's TIA framebuffer stores raw TIA colour codes (the COLUxx byte),
  // exactly like the Gopher2600 adapter's pixels, so we embed them directly and
  // pair them with the SUITE's canonical NTSC palette for an oracle-independent
  // golden PNG. Width is fixed 160 (H_PIXEL); height is the last full frame's
  // rendered scanline count.
  if (wantFrame) {
    // We drive the CPU directly, so the frontend's frame-render step never
    // runs; publish the latest completed frame (front buffer) into myFramebuffer
    // ourselves. onFrameComplete() fills the front buffer during stepping.
    tia.renderToFrameBuffer();
    const uInt8* fb = tia.frameBuffer();
    uInt16 width = 160;
    uInt16 height = (uInt16)tia.frameBufferScanlinesLastFrame();
    if (fb && height > 0) {
      // Roll so row 0 = the top of the field (VSYNC start). Stella's buffer is
      // YStart-centred: framebuffer row 0 sits startLine() scanlines below the
      // field top, so rolling up by startLine() restores the field origin. This
      // makes the full field comparable across oracles.
      int anchorRow = ((int)height - (int)tia.startLine()) % (int)height;
      std::vector<uint8_t> rolled((size_t)width * height);
      int a = anchorRow % height; if (a < 0) a += height;
      for (int r = 0; r < height; r++) {
        int src = (a + r) % height;
        std::memcpy(&rolled[(size_t)r * width], fb + (size_t)src * width, width);
      }
      const uint8_t* pal = (spec.rfind("PAL", 0) == 0) ? canonicalPALPalette
                                                        : canonicalNTSCPalette;
      gbtrace_writer_mark_frame_indexed(w, width, height, 12.0f / 7.0f,
          pal, 256, rolled.data(), (size_t)width * (size_t)height);
    }
  }

  if (gbtrace_writer_close(w) != 0) {
    std::fprintf(stderr, "error: writer close failed\n");
    return 1;
  }
  return 0;
}
