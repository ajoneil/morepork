// gbtrace-sameboy: Adapter that uses SameBoy to produce .gbtrace files.
//
// Links against libsameboy without any source modifications.
// Uses the public GB_set_execution_callback API to capture per-instruction
// CPU state, and GB_safe_read_memory (peek) for IO registers.
//
// Usage:
//   gbtrace-sameboy --rom test.gb --profile cpu_basic.toml --output trace.gbtrace
//
// Build:
//   See Makefile in this directory.

// Include C++ headers first to avoid conflicts with SameBoy's `internal` macro
// (defs.h redefines `internal` as a visibility attribute, which clashes with
// std::ios_base::internal). Also, debugger.h uses `new` as a parameter name.
#include <cctype>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <unistd.h>
#include <utility>
#include <sstream>
#include <string>
#include <unordered_map>
#include <vector>

#include "gbtrace.h"

// We define GB_INTERNAL to get full struct access (ime, cycles_since_run, etc.)
#define GB_INTERNAL
// Avoid C++ keyword conflict in debugger.h
#define new new_value

extern "C" {
#include <gb.h>
#include <memory.h>
}

#undef new

// --- Field configuration ---

// Map of field name -> IO register address for fields read via GB_safe_read_memory.
static const std::unordered_map<std::string, unsigned short> IO_FIELD_ADDR = {
    {"lcdc", 0xFF40}, {"stat", 0xFF41}, {"scy",  0xFF42}, {"scx",  0xFF43},
    {"ly",   0xFF44}, {"lyc",  0xFF45}, {"wy",   0xFF4A}, {"wx",   0xFF4B},
    {"bgp",  0xFF47}, {"obp0", 0xFF48}, {"obp1", 0xFF49}, {"dma",  0xFF46},
    {"div",  0xFF04}, {"tima", 0xFF05}, {"tma",  0xFF06}, {"tac",  0xFF07},
    {"if_",  0xFF0F}, {"ie",   0xFFFF},
    {"sb",   0xFF01}, {"sc",   0xFF02},
    // APU registers
    {"ch1_sweep", 0xFF10}, {"ch1_duty_len", 0xFF11}, {"ch1_vol_env", 0xFF12},
    {"ch1_freq_lo", 0xFF13}, {"ch1_freq_hi", 0xFF14},
    {"ch2_duty_len", 0xFF16}, {"ch2_vol_env", 0xFF17},
    {"ch2_freq_lo", 0xFF18}, {"ch2_freq_hi", 0xFF19},
    {"ch3_dac", 0xFF1A}, {"ch3_len", 0xFF1B}, {"ch3_vol", 0xFF1C},
    {"ch3_freq_lo", 0xFF1D}, {"ch3_freq_hi", 0xFF1E},
    {"ch4_len", 0xFF20}, {"ch4_vol_env", 0xFF21},
    {"ch4_freq", 0xFF22}, {"ch4_control", 0xFF23},
    {"master_vol", 0xFF24}, {"sound_pan", 0xFF25}, {"sound_on", 0xFF26},
};

// CPU register fields: maps field name -> register enum + is_16bit.
struct RegisterField {
    enum Reg { AF, BC, DE, HL, SP, PC,
               A, F, B, C, D, E, H, L };
    Reg reg;
    bool is_16bit;
};

static const std::unordered_map<std::string, RegisterField> REGISTER_FIELDS = {
    // SameBoy's execution callback fires per-instruction with the opcode
    // address, which both pc and op_addr (instruction address) emit.
    {"pc", {RegisterField::PC, true}},  {"op_addr", {RegisterField::PC, true}},
    {"sp", {RegisterField::SP, true}},
    {"a",  {RegisterField::A, false}},  {"f",  {RegisterField::F, false}},
    {"b",  {RegisterField::B, false}},  {"c",  {RegisterField::C, false}},
    {"d",  {RegisterField::D, false}},  {"e",  {RegisterField::E, false}},
    {"h",  {RegisterField::H, false}},  {"l",  {RegisterField::L, false}},
};

// --- Profile ---

struct Profile {
    std::string name;
    std::string trigger;
    std::vector<std::string> fields; // ordered
    std::unordered_map<std::string, unsigned short> memory; // name -> address
};

static Profile load_profile(const std::string &path) {
    GbtraceProfile *p = gbtrace_profile_load(path.c_str());
    if (!p) {
        std::fprintf(stderr, "Error: cannot load profile '%s'\n", path.c_str());
        std::exit(1);
    }

    Profile prof;
    prof.name = gbtrace_profile_name(p);
    prof.trigger = gbtrace_profile_trigger(p);

    size_t nfields = gbtrace_profile_num_fields(p);
    for (size_t i = 0; i < nfields; i++) {
        prof.fields.push_back(gbtrace_profile_field_name(p, i));
    }

    size_t nmem = gbtrace_profile_num_memory(p);
    for (size_t i = 0; i < nmem; i++) {
        prof.memory[gbtrace_profile_memory_name(p, i)] = gbtrace_profile_memory_addr(p, i);
    }

    gbtrace_profile_free(p);
    return prof;
}

// --- Globals for trace callback context ---

static GB_gameboy_t *g_gb = nullptr;
static Profile g_profile;
static uint64_t g_total_8mhz_ticks = 0; // needed for frame timing

static unsigned char g_stop_serial_byte = 0;
static int g_stop_serial_count = 1;
static int g_stop_serial_seen = 0;
static bool g_stop_serial_active = false;
static bool g_stop_serial_triggered = false;
static int g_stop_opcode = -1;
static bool g_stop_opcode_triggered = false;

// --- FFI writer ---
static GbtraceWriter *g_writer = nullptr;
static std::vector<int> g_writer_cols;

// Pre-computed list of what to emit per entry.
struct FieldEmitter {
    std::string name;
    enum Source { REGISTER_8, REGISTER_16, IO_READ, IME, PIX, OP_ADDR } source;
    RegisterField::Reg reg; // for REGISTER_8/16
    unsigned short io_addr; // for IO_READ
};
static std::vector<FieldEmitter> g_emitters;
static bool g_has_pix = false;
static uint32_t g_pixel_buf[160 * 144];
static std::string g_pending_pix;

// --- T-cycle (sub-instruction) tracing state ---
// When the profile's trigger is "tcycle" we emit one entry per emulated
// T-cycle via SameBoy's per-T-cycle callback (see the patch adding
// GB_set_tcycle_callback) rather than once per instruction. In that mode `pc`
// carries the live (mid-instruction) program counter while `op_addr` carries
// the stable address of the in-flight instruction's opcode.
static bool g_tcycle_mode = false;
static uint16_t g_op_addr = 0;     // opcode address of the in-flight instruction
static int g_frames = 0;           // frame counter (shared with the callback)
static uint8_t g_prev_stat_mode = 0xFF; // for vblank-edge detection in tcycle mode
static bool g_has_reference = false;    // a reference image is loaded

static inline char rgba_to_shade(uint32_t rgba) {
    unsigned r = (rgba >> 0) & 0xFF;
    if (r >= 0xC0) return '0';
    if (r >= 0x70) return '1';
    if (r >= 0x30) return '2';
    return '3';
}

// CGB output is colour, so the pix field stores RGB555 (4 hex chars/pixel)
// rather than a 2-bit greyscale shade. Set once the model is known.
static bool g_cgb = false;

static void capture_sameboy_frame() {
    g_pending_pix.clear();
    g_pending_pix.reserve(160 * 144 * (g_cgb ? 4 : 1));
    for (int i = 0; i < 160 * 144; i++) {
        uint32_t px = g_pixel_buf[i];  // byte0=R, byte1=G, byte2=B (see rgb_encode_callback)
        if (g_cgb) {
            unsigned r = px & 0xFF, g = (px >> 8) & 0xFF, b = (px >> 16) & 0xFF;
            unsigned v = ((r >> 3) << 10) | ((g >> 3) << 5) | (b >> 3);
            char hex[5];
            std::snprintf(hex, sizeof(hex), "%04X", v);
            g_pending_pix += hex;
        } else {
            g_pending_pix += rgba_to_shade(g_pixel_buf[i]);
        }
    }
}

// --- Reference matching ---
// References are raw RGB555 (160*144*3 bytes, each channel 0-31). Comparing
// at the CGB's native 5-bit precision is expansion-neutral.
static std::string g_reference;  // raw RGB555 bytes

static bool load_reference(const std::string &path) {
    std::ifstream f(path, std::ios::binary);
    if (!f.is_open()) return false;
    g_reference.assign(std::istreambuf_iterator<char>(f),
                       std::istreambuf_iterator<char>());
    return g_reference.size() == (size_t)(160 * 144 * 3);
}

static bool frame_matches_reference() {
    if (g_reference.size() != (size_t)(160 * 144 * 3)) return false;
    const unsigned char *ref = reinterpret_cast<const unsigned char *>(g_reference.data());
    for (int i = 0; i < 160 * 144; i++) {
        uint32_t px = g_pixel_buf[i];  // bytes: r, g, b, 0xFF
        int r = (int)((px >> 0) & 0xFF) >> 3;
        int g = (int)((px >> 8) & 0xFF) >> 3;
        int b = (int)((px >> 16) & 0xFF) >> 3;
        if (std::abs(r - ref[i * 3]) > 1 || std::abs(g - ref[i * 3 + 1]) > 1 ||
            std::abs(b - ref[i * 3 + 2]) > 1)
            return false;
    }
    return true;
}

static void build_emitters(const Profile &prof) {
    g_emitters.clear();
    for (const auto &field : prof.fields) {

        FieldEmitter em;
        em.name = field;

        if (field == "pix") {
            em.source = FieldEmitter::PIX;
            g_has_pix = true;
            g_emitters.push_back(em);
            continue;
        } else if (field == "op_addr") {
            // Instruction address (stable across the instruction's T-cycles).
            em.source = FieldEmitter::OP_ADDR;
        } else if (field == "ime") {
            em.source = FieldEmitter::IME;
        } else if (auto it = REGISTER_FIELDS.find(field); it != REGISTER_FIELDS.end()) {
            em.source = it->second.is_16bit ? FieldEmitter::REGISTER_16 : FieldEmitter::REGISTER_8;
            em.reg = it->second.reg;
        } else if (auto it2 = IO_FIELD_ADDR.find(field); it2 != IO_FIELD_ADDR.end()) {
            em.source = FieldEmitter::IO_READ;
            em.io_addr = it2->second;
        } else if (auto it3 = prof.memory.find(field); it3 != prof.memory.end()) {
            em.source = FieldEmitter::IO_READ;
            em.io_addr = it3->second;
        } else {
            std::fprintf(stderr, "Warning: unknown field '%s', skipping\n", field.c_str());
            continue;
        }
        g_emitters.push_back(em);
    }
}

// Read a register value from the emulator.
static inline int read_reg(GB_gameboy_t *gb, RegisterField::Reg reg) {
    GB_registers_t *regs = GB_get_registers(gb);
    switch (reg) {
    case RegisterField::AF: return regs->af;
    case RegisterField::BC: return regs->bc;
    case RegisterField::DE: return regs->de;
    case RegisterField::HL: return regs->hl;
    case RegisterField::SP: return regs->sp;
    case RegisterField::PC: return regs->pc;
    case RegisterField::A:  return regs->a;
    case RegisterField::F:  return regs->f;
    case RegisterField::B:  return regs->b;
    case RegisterField::C:  return regs->c;
    case RegisterField::D:  return regs->d;
    case RegisterField::E:  return regs->e;
    case RegisterField::H:  return regs->h;
    case RegisterField::L:  return regs->l;
    }
    return 0;
}

// --- Emit entry ---

static void emit_entry(GB_gameboy_t *gb, uint16_t address) {
    // Set all field values
    for (size_t i = 0; i < g_emitters.size(); i++) {
        int col = g_writer_cols[i];
        if (col < 0) continue;
        const auto &em = g_emitters[i];
        switch (em.source) {
        case FieldEmitter::REGISTER_8:
            gbtrace_writer_set_u8(g_writer, col, read_reg(gb, em.reg));
            break;
        case FieldEmitter::REGISTER_16:
            if (em.reg == RegisterField::PC)
                // Instruction mode: pc is the opcode address (== op_addr).
                // T-cycle mode: pc is the live, mid-instruction program counter.
                gbtrace_writer_set_u16(g_writer, col,
                                       g_tcycle_mode ? GB_get_registers(gb)->pc : address);
            else
                gbtrace_writer_set_u16(g_writer, col, read_reg(gb, em.reg));
            break;
        case FieldEmitter::OP_ADDR:
            gbtrace_writer_set_u16(g_writer, col, address);
            break;
        case FieldEmitter::IO_READ:
            gbtrace_writer_set_u8(g_writer, col, GB_safe_read_memory(gb, em.io_addr));
            break;
        case FieldEmitter::IME:
            gbtrace_writer_set_bool(g_writer, col, gb->ime);
            break;
        case FieldEmitter::PIX:
            gbtrace_writer_set_str(g_writer, col,
                                   g_pending_pix.c_str(), g_pending_pix.size());
            g_pending_pix.clear();
            break;
        }
    }

    gbtrace_writer_finish_entry(g_writer);
}

// --- Trace callback ---

static void exec_callback(GB_gameboy_t *gb, uint16_t address, uint8_t opcode) {
    // Record the in-flight instruction's opcode address. In instruction mode we
    // also emit the entry here; in T-cycle mode emission happens per T-cycle in
    // tcycle_callback (this only updates op_addr / checks stop conditions).
    g_op_addr = address;
    if (!g_tcycle_mode) {
        emit_entry(gb, address);
    }

    // Check opcode stop condition
    if (g_stop_opcode >= 0 && !g_stop_opcode_triggered) {
        if (opcode == static_cast<uint8_t>(g_stop_opcode)) {
            g_stop_opcode_triggered = true;
        }
    }

    // Check serial stop condition: detect rising edge of SC bit 7
    if (g_stop_serial_active && !g_stop_serial_triggered) {
        static bool prev_sc_high = false;
        unsigned char sc = GB_safe_read_memory(gb, 0xFF02);
        bool sc_high = (sc & 0x80) != 0;
        if (sc_high && !prev_sc_high) {
            unsigned char sb = GB_safe_read_memory(gb, 0xFF01);
            if (sb == g_stop_serial_byte) {
                g_stop_serial_seen++;
                if (g_stop_serial_seen >= g_stop_serial_count) {
                    g_stop_serial_triggered = true;
                }
            }
        }
        prev_sc_high = sc_high;
    }
}

// --- Per-T-cycle trace callback ---
// Fires once per emulated T-cycle (SameBoy patch: GB_set_tcycle_callback).
// Detects the vblank edge at T-cycle precision to capture the framebuffer and
// mark the frame boundary, then emits one entry for this T-cycle. `pc` is the
// live program counter; `op_addr` is the in-flight instruction address.
static void tcycle_callback(GB_gameboy_t *gb) {
    uint8_t mode = GB_safe_read_memory(gb, 0xFF41) & 3;
    if (mode == 1 && g_prev_stat_mode != 1) {
        g_frames++;
        if (g_has_pix || g_has_reference) {
            capture_sameboy_frame();
        }
        gbtrace_writer_mark_frame(g_writer);
    }
    g_prev_stat_mode = mode;

    emit_entry(gb, g_op_addr);
}

// --- SHA-256 ---

static std::string sha256_file(const std::string &path) {
    std::string cmd = "sha256sum \"" + path + "\"";
    FILE *pipe = popen(cmd.c_str(), "r");
    if (!pipe) return "unknown";
    char buf[128];
    std::string result;
    if (std::fgets(buf, sizeof(buf), pipe)) {
        result = buf;
        auto space = result.find(' ');
        if (space != std::string::npos)
            result = result.substr(0, space);
    }
    pclose(pipe);
    return result;
}

// --- Stop condition ---

struct StopCondition {
    unsigned short addr;
    unsigned char value;
    bool negate = false;
    bool active = false;
};

static StopCondition parse_stop_when(const std::string &spec) {
    auto neq = spec.find("!=");
    auto eq = spec.find('=');
    if (eq == std::string::npos) {
        std::fprintf(stderr, "Error: --stop-when format is ADDR=VAL or ADDR!=VAL (e.g. A000!=80)\n");
        std::exit(1);
    }
    StopCondition cond;
    bool is_negate = (neq != std::string::npos && neq < eq);
    cond.addr = static_cast<unsigned short>(std::strtoul(spec.substr(0, is_negate ? neq : eq).c_str(), nullptr, 16));
    cond.value = static_cast<unsigned char>(std::strtoul(spec.substr(eq + 1).c_str(), nullptr, 16));
    cond.negate = is_negate;
    cond.active = true;
    return cond;
}

// --- Main ---

// --- Audio capture (gambatte _outaudio tests) ---
// SameBoy emits stereo int16 samples via the APU sample callback. A
// frame is "silent" when every sample matches the first; tolerance
// ~0.005 of full-scale (matching missingno) absorbs APU DC drift.
static std::vector<std::pair<int16_t, int16_t>> g_audio;
static void audio_callback(GB_gameboy_t *, GB_sample_t *s) {
    g_audio.emplace_back(s->left, s->right);
}
static bool last_frame_has_audio(int frames) {
    if (g_audio.empty() || frames <= 0) return false;
    size_t per_frame = g_audio.size() / static_cast<size_t>(frames);
    if (per_frame == 0) per_frame = g_audio.size();
    size_t start = g_audio.size() - per_frame;
    int16_t l0 = g_audio[start].first, r0 = g_audio[start].second;
    for (size_t i = start; i < g_audio.size(); i++) {
        if (std::abs(g_audio[i].first - l0) > 163 ||
            std::abs(g_audio[i].second - r0) > 163)
            return true;
    }
    return false;
}

static void print_usage(const char *argv0) {
    std::fprintf(stderr,
        "Usage: %s --rom <file.gb> --profile <profile.toml> [options]\n"
        "\n"
        "Options:\n"
        "  --rom <path>         ROM file to run (required)\n"
        "  --profile <path>     Capture profile TOML file (required)\n"
        "  --output <path>      Output trace file (required)\n"
        "  --frames <n>         Stop after N frames (default: 3000)\n"
        "  --stop-when <A=V>    Stop when memory ADDR equals VAL (hex, e.g. A000=80)\n"
        "  --stop-on-serial <HH>  Stop when serial byte HH is sent (hex)\n"
        "  --stop-serial-count <n> Require n serial matches before stopping (default: 1)\n"
        "  --model <model>      dmg or cgb (default: dmg)\n"
        "  --report-audio       print AUDIO=0/1 (last-frame activity) for _outaudio tests\n"
        "  --boot-rom <path>    Boot ROM file (default: boot_roms/<model>_boot.bin)\n",
        argv0);
}

int main(int argc, char *argv[]) {
    std::string rom_path;
    std::string profile_path;
    std::string output_path;
    std::string boot_rom_path;
    int max_frames = 3000;
    long until_tcycle = -1;  // >=0: run N T-cycles, capture final screen
    std::string model = "DMG-B";
    std::string reference_path;
    int extra_frames = 0;
    bool report_audio = false;
    GB_model_t gb_model = GB_MODEL_DMG_B;
    std::vector<StopCondition> stop_conditions;

    for (int i = 1; i < argc; i++) {
        std::string arg = argv[i];
        if (arg == "--rom" && i + 1 < argc) {
            rom_path = argv[++i];
        } else if (arg == "--profile" && i + 1 < argc) {
            profile_path = argv[++i];
        } else if (arg == "--output" && i + 1 < argc) {
            output_path = argv[++i];
        } else if (arg == "--frames" && i + 1 < argc) {
            max_frames = std::atoi(argv[++i]);
        } else if (arg == "--until-tcycle" && i + 1 < argc) {
            until_tcycle = std::atol(argv[++i]);
        } else if (arg == "--stop-when" && i + 1 < argc) {
            stop_conditions.push_back(parse_stop_when(argv[++i]));
        } else if (arg == "--stop-on-serial" && i + 1 < argc) {
            g_stop_serial_byte = static_cast<unsigned char>(
                std::strtoul(argv[++i], nullptr, 16));
            g_stop_serial_active = true;
        } else if (arg == "--stop-serial-count" && i + 1 < argc) {
            g_stop_serial_count = std::atoi(argv[++i]);
        } else if (arg == "--boot-rom" && i + 1 < argc) {
            boot_rom_path = argv[++i];
        } else if (arg == "--model" && i + 1 < argc) {
            std::string m = argv[++i];
            for (auto &c : m) c = std::tolower(c);
            // CGB SoC revision is selectable. Plain "cgb" defaults to CGB-C
            // (Gambatte's cgb04c) so cross-emulator CGB diffs line up with
            // gambatte/missingno; specific revisions can be requested too.
            if (m == "dmg") { model = "DMG-B"; gb_model = GB_MODEL_DMG_B; }
            else if (m == "cgb" || m == "cgb-c" || m == "cgb04c") { model = "CGB-C"; gb_model = GB_MODEL_CGB_C; }
            else if (m == "cgb-0" || m == "cgb0") { model = "CGB-0"; gb_model = GB_MODEL_CGB_0; }
            else if (m == "cgb-a") { model = "CGB-A"; gb_model = GB_MODEL_CGB_A; }
            else if (m == "cgb-b") { model = "CGB-B"; gb_model = GB_MODEL_CGB_B; }
            else if (m == "cgb-d") { model = "CGB-D"; gb_model = GB_MODEL_CGB_D; }
            else if (m == "cgb-e") { model = "CGB-E"; gb_model = GB_MODEL_CGB_E; }
            else if (m == "agb") { model = "AGB-A"; gb_model = GB_MODEL_AGB; }
            else { std::fprintf(stderr, "Warning: unknown --model '%s', using DMG-B\n", m.c_str()); }
        } else if (arg == "--reference" && i + 1 < argc) {
            reference_path = argv[++i];
        } else if (arg == "--extra-frames" && i + 1 < argc) {
            extra_frames = std::atoi(argv[++i]);
        } else if (arg == "--stop-opcode" && i + 1 < argc) {
            g_stop_opcode = static_cast<int>(std::strtoul(argv[++i], nullptr, 16));
        } else if (arg == "--report-audio") {
            report_audio = true;
        } else if (arg == "--help" || arg == "-h") {
            print_usage(argv[0]);
            return 0;
        }
    }

    if (rom_path.empty() || profile_path.empty() || output_path.empty()) {
        print_usage(argv[0]);
        return 1;
    }

    // Default boot ROM: resolve relative to executable location
    if (boot_rom_path.empty()) {
        // Find directory containing the executable
        std::string exe_dir;
        char exe_buf[4096];
        ssize_t len = readlink("/proc/self/exe", exe_buf, sizeof(exe_buf) - 1);
        if (len > 0) {
            exe_buf[len] = '\0';
            exe_dir = exe_buf;
            auto slash = exe_dir.rfind('/');
            if (slash != std::string::npos)
                exe_dir = exe_dir.substr(0, slash);
        } else {
            exe_dir = ".";
        }

        bool is_cgb = (gb_model & GB_MODEL_FAMILY_MASK) == GB_MODEL_CGB_FAMILY;
        std::string boot_name = is_cgb ? "cgb_boot.bin" : "dmg_boot.bin";
        boot_rom_path = exe_dir + "/boot_roms/" + boot_name;
    }

    // Load profile
    g_profile = load_profile(profile_path);
    build_emitters(g_profile);

    // T-cycle granularity is honoured via SameBoy's per-T-cycle callback; any
    // other trigger falls back to per-instruction emission.
    g_tcycle_mode = (g_profile.trigger == "tcycle");

    std::fprintf(stderr, "Profile: %s (%zu fields)\n",
                 g_profile.name.c_str(), g_profile.fields.size());

    // Init emulator
    g_gb = GB_init(GB_alloc(), gb_model);

    // Load boot ROM
    int bios_result = GB_load_boot_rom(g_gb, boot_rom_path.c_str());
    if (bios_result != 0) {
        std::fprintf(stderr, "Error: failed to load boot ROM '%s' (error %d)\n",
                     boot_rom_path.c_str(), bios_result);
        return 1;
    }
    std::string boot_rom_info = sha256_file(boot_rom_path);
    std::fprintf(stderr, "Boot ROM: %s (sha256: %s)\n",
                 boot_rom_path.c_str(), boot_rom_info.c_str());

    int load_result = GB_load_rom(g_gb, rom_path.c_str());
    if (load_result != 0) {
        std::fprintf(stderr, "Error: failed to load ROM '%s' (error %d)\n",
                     rom_path.c_str(), load_result);
        return 1;
    }

    // Optimizations for trace generation
    bool need_pixels = g_has_pix || !reference_path.empty();
    if (!need_pixels) {
        GB_set_rendering_disabled(g_gb, true);
    } else {
        GB_set_pixels_output(g_gb, g_pixel_buf);
        GB_set_color_correction_mode(g_gb, GB_COLOR_CORRECTION_DISABLED);
        // DMG only: use the greyscale palette (SameBoy defaults to a green
        // tint) so DMG screenshots match the greyscale reference images.
        // Must NOT be set for CGB — it overrides the ROM's colour palettes
        // and would turn colour CGB output greyscale.
        if ((gb_model & GB_MODEL_FAMILY_MASK) != GB_MODEL_CGB_FAMILY) {
            GB_set_palette(g_gb, &GB_PALETTE_GREY);
        }
        // Set RGB encode callback so pixel buffer gets standard 0xRRGGBB values
        GB_set_rgb_encode_callback(g_gb, [](GB_gameboy_t *, uint8_t r, uint8_t g, uint8_t b) -> uint32_t {
            return (uint32_t)r | ((uint32_t)g << 8) | ((uint32_t)b << 16) | 0xFF000000u;
        });
    }
    GB_set_turbo_mode(g_gb, true, true);

    // Run boot ROM without tracing until it unmaps itself. The boot ROM's
    // final act is writing $FF50 to disable itself (latched, read back as
    // 0xFE | finished) before handing off to the cartridge at $0100.
    //
    // We must NOT stop at "pc >= 0x0100": the CGB boot ROM is mapped at both
    // $0000-$00FF and $0200-$08FF, so its own code executes above $0100 partway
    // through boot — stopping there would begin tracing *inside* the CGB boot
    // ROM (the logo/animation), which is what was leaking into CGB traces.
    std::fprintf(stderr, "Running boot ROM (no trace)...\n");
    const uint64_t BOOT_TICK_CAP = 100000000ull;  // ~12s emulated; real boot is far shorter
    while ((GB_safe_read_memory(g_gb, 0xFF50) & 1) == 0) {
        g_total_8mhz_ticks += GB_run(g_gb);
        if (g_total_8mhz_ticks > BOOT_TICK_CAP) {
            std::fprintf(stderr, "Warning: boot ROM did not finish within %llu ticks\n",
                         (unsigned long long)BOOT_TICK_CAP);
            break;
        }
    }
    std::fprintf(stderr, "Boot complete at cycle %llu (pc=0x%04X)\n",
                 (unsigned long long)(g_total_8mhz_ticks / 2),
                 GB_get_registers(g_gb)->pc);

    // Reset cycle origin so traces start at cy=0 post-boot
    g_total_8mhz_ticks = 0;

    // CGB output is colour → store the pix field as RGB555.
    g_cgb = (gb_model & GB_MODEL_FAMILY_MASK) == GB_MODEL_CGB_FAMILY;

    // Init FFI writer
    std::string rom_hash = sha256_file(rom_path);

    // Build header JSON for the FFI writer
    std::string pix_format = g_cgb ? "\"pix_format\":\"rgb555\"," : "";
    std::string header_json = "{\"_header\":true,\"format_version\":\"0.1.0\","
        "\"emulator\":\"sameboy\",\"emulator_version\":\"0.16.x\","
        "\"rom_sha256\":\"" + rom_hash + "\",\"model\":\"" + model + "\","
        "\"boot_rom\":\"" + boot_rom_info + "\",\"profile\":\"" + g_profile.name + "\","
        + pix_format + "\"fields\":[";
    for (size_t i = 0; i < g_emitters.size(); i++) {
        if (i > 0) header_json += ",";
        header_json += "\"" + g_emitters[i].name + "\"";
    }
    header_json += "],\"trigger\":\"";
    header_json += g_tcycle_mode ? "tcycle" : "instruction";
    header_json += "\"}";

    g_writer = gbtrace_writer_new(
        output_path.c_str(), header_json.c_str(), header_json.size());
    if (!g_writer) {
        std::fprintf(stderr, "Error: failed to create writer for '%s'\n",
                     output_path.c_str());
        return 1;
    }

    // Cache column indices
    g_writer_cols.resize(g_emitters.size());
    for (size_t i = 0; i < g_emitters.size(); i++) {
        g_writer_cols[i] = gbtrace_writer_find_field(
            g_writer, g_emitters[i].name.c_str());
    }

    // Mark entry 0 as a frame boundary
    gbtrace_writer_mark_frame(g_writer);

    GB_set_execution_callback(g_gb, exec_callback);
    if (g_tcycle_mode) {
        // Emit one entry per emulated T-cycle rather than per instruction.
        GB_set_tcycle_callback(g_gb, tcycle_callback);
    }

    // Capture audio only when asked (gambatte _outaudio tests). Set up
    // after boot so boot-time audio isn't measured.
    if (report_audio) {
        GB_set_sample_rate(g_gb, 44100);
        GB_apu_set_sample_callback(g_gb, audio_callback);
        g_audio.clear();
    }

    // Run: GB_run executes one CPU step and returns 8MHz ticks consumed.
    for (const auto &cond : stop_conditions) {
        std::fprintf(stderr, "Stop condition: [0x%04X] == 0x%02X\n",
                     cond.addr, cond.value);
    }
    if (g_stop_serial_active) {
        std::fprintf(stderr, "Serial stop: byte=0x%02X count=%d\n",
                     g_stop_serial_byte, g_stop_serial_count);
    }

    // Load reference image
    bool has_reference = false;
    if (!reference_path.empty()) {
        if (load_reference(reference_path)) {
            has_reference = true;
            g_has_reference = true;
            std::fprintf(stderr, "Reference: %s (%d pixels)\n",
                         reference_path.c_str(), 160 * 144);
        } else {
            std::fprintf(stderr, "Warning: could not load reference '%s'\n",
                         reference_path.c_str());
        }
    }

    int &frames = g_frames;  // shared with tcycle_callback (incremented there in T-cycle mode)
    bool stopped_early = false;
    int remaining_extra = -1;  // -1 = not triggered yet
    // Cycle-budget mode (gambatte tests): run for exactly N T-cycles, then
    // snapshot the screen — matching the gambatte testrunner, which reads the
    // framebuffer after a fixed cycle budget rather than counting vblank events.
    // g_total_8mhz_ticks counts 8 MHz ticks (== 2 T-cycles); it was reset to 0
    // after the boot ROM, so the budget is measured from the cartridge entry.
    const bool cycle_budget = until_tcycle >= 0;
    const uint64_t budget_ticks = (uint64_t)until_tcycle * 2;
    while (cycle_budget ? (g_total_8mhz_ticks < budget_ticks) : (frames < max_frames)) {
        unsigned ticks = GB_run(g_gb);
        g_total_8mhz_ticks += ticks;
        if (g_gb->vblank_just_occured) {
            // In T-cycle mode the callback already counted this frame, captured
            // the framebuffer and marked the boundary at the exact vblank
            // T-cycle; only the instruction-mode path does it here.
            if (!g_tcycle_mode) {
                frames++;
                if (g_has_pix || has_reference) {
                    capture_sameboy_frame();
                }
                gbtrace_writer_mark_frame(g_writer);
            }

            // Check reference match (immediate stop)
            if (has_reference && frame_matches_reference()) {
                std::fprintf(stderr, "Reference match at frame %d\n", frames);
                while (true) {
                    GB_run(g_gb);
                    if (g_gb->vblank_just_occured) break;
                }
                stopped_early = true;
                break;
            }

            // If in extra-frames countdown, just decrement
            if (remaining_extra >= 0) {
                if (remaining_extra == 0) {
                    stopped_early = true;
                    break;
                }
                remaining_extra--;
                continue;
            }

            // Check stop conditions — start countdown
            for (const auto &cond : stop_conditions) {
                bool match = GB_safe_read_memory(g_gb, cond.addr) == cond.value;
                if (cond.negate ? !match : match) {
                    std::fprintf(stderr, "Stop condition met at frame %d, running %d extra frame%s\n",
                                 frames, extra_frames, extra_frames == 1 ? "" : "s");
                    remaining_extra = extra_frames;
                    break;
                }
            }
            if (remaining_extra >= 0 && remaining_extra == 0) {
                stopped_early = true;
                break;
            }
            if (g_stop_serial_triggered) {
                std::fprintf(stderr, "Serial stop at frame %d, running %d extra frame%s\n",
                             frames, extra_frames, extra_frames == 1 ? "" : "s");
                remaining_extra = extra_frames;
                if (remaining_extra == 0) {
                    stopped_early = true;
                    break;
                }
            }
            if (g_stop_opcode_triggered) {
                std::fprintf(stderr, "Opcode stop at frame %d, running %d extra frame%s\n",
                             frames, extra_frames, extra_frames == 1 ? "" : "s");
                remaining_extra = extra_frames;
                if (remaining_extra == 0) {
                    stopped_early = true;
                    break;
                }
            }
        }
    }

    // Cycle-budget mode: emit the full framebuffer at the budget as the trace's
    // final frame (mirrors the per-vblank capture above). This is the screen the
    // gambatte hex/blank check reads, regardless of where we stopped in a frame.
    if (cycle_budget) {
        if (g_has_pix || has_reference) capture_sameboy_frame();
        gbtrace_writer_mark_frame(g_writer);
        emit_entry(g_gb, g_tcycle_mode ? g_op_addr : GB_get_registers(g_gb)->pc);
    }

    if (report_audio) {
        std::fprintf(stderr, "AUDIO=%d\n", last_frame_has_audio(frames) ? 1 : 0);
    }

    gbtrace_writer_close(g_writer);
    g_writer = nullptr;

    GB_free(g_gb);
    GB_dealloc(g_gb);

    if (stopped_early) {
        std::fprintf(stderr, "Stop condition met at frame %d.\n", frames);
    }
    std::fprintf(stderr, "Traced %d frames, output written.\n", frames);
    return 0;
}
