// gbtrace-gambatte: Adapter that uses libgambatte to produce .gbtrace files.
//
// Links against libgambatte (gambatte-speedrun) without any source modifications.
// Uses the public traceCallback API to capture per-instruction CPU state,
// and externalRead (peek) for IO registers (PPU, timer, interrupts).
//
// Usage:
//   gbtrace-gambatte --rom test.gb --profile cpu_basic.toml --output trace.gbtrace
//
// Build:
//   See Makefile in this directory.

#include <gambatte.h>
#include "gbtrace.h"

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <sstream>
#include <string>
#include <unordered_map>
#include <vector>

// --- Field configuration ---

// Map of field name -> IO register address for fields read via externalRead.
// CPU register fields are read from the trace callback data array instead.
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

// Fields available from the trace callback data array.
// Maps field name -> (array index, is_16bit).
struct CallbackField { int index; bool is_16bit; };
static const std::unordered_map<std::string, CallbackField> CALLBACK_FIELDS = {
    // gambatte traces per-instruction, so op_addr (instruction address)
    // equals pc — both read callback slot 1.
    {"pc", {1, true}},  {"op_addr", {1, true}},  {"sp", {2, true}},
    {"a",  {3, false}}, {"b",  {4, false}}, {"c",  {5, false}},
    {"d",  {6, false}}, {"e",  {7, false}}, {"f",  {8, false}},
    {"h",  {9, false}}, {"l",  {10, false}},
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

static gambatte::GB *g_gb = nullptr;
static Profile g_profile;
static unsigned char g_stop_serial_byte = 0;
static int g_stop_serial_count = 1;  // stop after Nth occurrence
static int g_stop_serial_seen = 0;
static bool g_stop_serial_active = false;
static bool g_stop_serial_triggered = false;

// Pre-computed list of what to emit per entry, for fast callback execution.
struct FieldEmitter {
    std::string name;
    enum Source { CALLBACK_8, CALLBACK_16, IO_READ, IME, PIX } source;
    int cb_index;           // for CALLBACK_8/16
    unsigned short io_addr; // for IO_READ
};
static std::vector<FieldEmitter> g_emitters;
static bool g_has_pix = false;
static int g_stop_opcode = -1;
static bool g_stop_opcode_triggered = false;

// --- FFI writer ---
static GbtraceWriter *g_writer = nullptr;
static std::vector<int> g_writer_cols;

// --- Pixel capture ---
// Gambatte fills video_buf as a 160x144 RGBA framebuffer during runFor().
// After each frame completes, we convert to a 2-bit shade string and emit
// it on the next trace entry. Pixels accumulate in g_pending_pix.
static gambatte::uint_least32_t *g_video_buf_ptr = nullptr;
static std::string g_pending_pix;
// CGB output is colour, so the pix field stores RGB555 (4 hex chars/pixel)
// rather than a 2-bit greyscale shade. Set once the model is known.
static bool g_cgb = false;

static inline char rgba_to_shade_char(gambatte::uint_least32_t rgba) {
    // Use red channel — gambatte's default greyscale palette
    unsigned r = rgba & 0xFF;
    if (r >= 0xC0) return '0';
    if (r >= 0x70) return '1';
    if (r >= 0x30) return '2';
    return '3';
}

static void capture_frame_pixels() {
    if (!g_video_buf_ptr) return;
    g_pending_pix.clear();
    g_pending_pix.reserve(160 * 144 * (g_cgb ? 4 : 1));
    for (int i = 0; i < 160 * 144; i++) {
        gambatte::uint_least32_t px = g_video_buf_ptr[i];  // 0x00RRGGBB
        if (g_cgb) {
            unsigned r = (px >> 16) & 0xFF, g = (px >> 8) & 0xFF, b = px & 0xFF;
            unsigned v = ((r >> 3) << 10) | ((g >> 3) << 5) | (b >> 3);
            char hex[5];
            std::snprintf(hex, sizeof(hex), "%04X", v);
            g_pending_pix += hex;
        } else {
            g_pending_pix += rgba_to_shade_char(px);
        }
    }
}

// --- Reference matching ---
// References are raw RGB555 (160*144*3 bytes, each channel 0-31). Comparing
// at the CGB's native 5-bit precision is expansion-neutral, so a correct
// emulator isn't penalised for its 555→888 display-expansion curve.
static std::string g_reference;  // raw RGB555 bytes

static bool load_reference(const std::string &path) {
    std::ifstream f(path, std::ios::binary);
    if (!f.is_open()) return false;
    g_reference.assign(std::istreambuf_iterator<char>(f),
                       std::istreambuf_iterator<char>());
    return g_reference.size() == (size_t)(160 * 144 * 3);
}

static bool frame_matches_reference() {
    if (g_reference.size() != (size_t)(160 * 144 * 3) || !g_video_buf_ptr) return false;
    const unsigned char *ref = reinterpret_cast<const unsigned char *>(g_reference.data());
    for (int i = 0; i < 160 * 144; i++) {
        // gambatte emits RGB32 (native endian) = 0x00RRGGBB.
        gambatte::uint_least32_t px = g_video_buf_ptr[i];
        int r = (int)((px >> 16) & 0xFF) >> 3;
        int g = (int)((px >> 8) & 0xFF) >> 3;
        int b = (int)(px & 0xFF) >> 3;
        if (std::abs(r - ref[i * 3]) > 1 || std::abs(g - ref[i * 3 + 1]) > 1 ||
            std::abs(b - ref[i * 3 + 2]) > 1)
            return false;
    }
    return true;
}

// --- Audio activity (gambatte _outaudio tests) ---
// gambatte packs each stereo sample as two signed 16-bit channels in a
// uint_least32_t (left in low 16 bits, right in high 16). A frame is
// "silent" when every sample matches the first; "has audio" otherwise.
// Tolerance ~0.005 of full-scale (matching missingno) absorbs APU DC drift.
static bool last_frame_has_audio(const gambatte::uint_least32_t *buf, std::size_t n) {
    if (n == 0) return false;
    int16_t l0 = static_cast<int16_t>(buf[0] & 0xFFFF);
    int16_t r0 = static_cast<int16_t>(buf[0] >> 16);
    for (std::size_t i = 1; i < n; i++) {
        int16_t l = static_cast<int16_t>(buf[i] & 0xFFFF);
        int16_t r = static_cast<int16_t>(buf[i] >> 16);
        if (std::abs(l - l0) > 163 || std::abs(r - r0) > 163) return true;
    }
    return false;
}

static void build_emitters(const Profile &prof) {
    g_emitters.clear();
    for (const auto &field : prof.fields) {
        FieldEmitter em;
        em.name = field;

        if (field == "ime") {
            // gambatte doesn't expose IME — skip rather than emit fake data
            std::fprintf(stderr, "Note: skipping 'ime' (not available in gambatte)\n");
            continue;
        } else if (field == "pix") {
            em.source = FieldEmitter::PIX;
            g_has_pix = true;
            g_emitters.push_back(em);
            continue;
        } else if (auto it = CALLBACK_FIELDS.find(field); it != CALLBACK_FIELDS.end()) {
            em.source = it->second.is_16bit ? FieldEmitter::CALLBACK_16 : FieldEmitter::CALLBACK_8;
            em.cb_index = it->second.index;
        } else if (auto it2 = IO_FIELD_ADDR.find(field); it2 != IO_FIELD_ADDR.end()) {
            em.source = FieldEmitter::IO_READ;
            em.io_addr = it2->second;
        } else if (auto it3 = prof.memory.find(field); it3 != prof.memory.end()) {
            em.source = FieldEmitter::IO_READ; // same mechanism — peek memory
            em.io_addr = it3->second;
        } else {
            std::fprintf(stderr, "Warning: unknown field '%s', skipping\n", field.c_str());
            continue;
        }
        g_emitters.push_back(em);
    }
}

// --- Trace callback ---

// Cache for IO values — used to emit pre-execution state.
// The callback fires AFTER the instruction executes, so externalRead()
// gives post-execution values. We cache the IO values and emit them
// on the NEXT callback, giving pre-execution state for that instruction.
static std::unordered_map<unsigned short, unsigned char> g_io_cache;
static bool g_io_cache_valid = false;

static void emit_entry(int *r) {
    // Read current IO values (post-execution of this instruction)
    std::unordered_map<unsigned short, unsigned char> io_now;
    for (const auto &em : g_emitters) {
        if (em.source == FieldEmitter::IO_READ) {
            io_now[em.io_addr] = g_gb->externalRead(em.io_addr);
        }
    }

    // Set all field values
    for (size_t i = 0; i < g_emitters.size(); i++) {
        int col = g_writer_cols[i];
        if (col < 0) continue;
        const auto &em = g_emitters[i];
        switch (em.source) {
        case FieldEmitter::CALLBACK_8:
            gbtrace_writer_set_u8(g_writer, col, r[em.cb_index] & 0xFF);
            break;
        case FieldEmitter::CALLBACK_16:
            gbtrace_writer_set_u16(g_writer, col, r[em.cb_index] & 0xFFFF);
            break;
        case FieldEmitter::IO_READ:
            if (g_io_cache_valid) {
                gbtrace_writer_set_u8(g_writer, col, g_io_cache[em.io_addr]);
            } else {
                gbtrace_writer_set_u8(g_writer, col, io_now[em.io_addr]);
            }
            break;
        case FieldEmitter::IME:
            break;
        case FieldEmitter::PIX:
            gbtrace_writer_set_str(g_writer, col,
                                   g_pending_pix.c_str(), g_pending_pix.size());
            g_pending_pix.clear();
            break;
        }
    }

    gbtrace_writer_finish_entry(g_writer);

    // Update cache for next callback
    g_io_cache = io_now;
    g_io_cache_valid = true;
}

static void trace_callback(void *data) {
    int *r = static_cast<int *>(data);

    emit_entry(r);

    // Check opcode stop condition
    if (g_stop_opcode >= 0 && !g_stop_opcode_triggered) {
        unsigned pc = r[1];
        if (g_gb->externalRead(pc) == static_cast<unsigned>(g_stop_opcode)) {
            g_stop_opcode_triggered = true;
        }
    }

    // Check serial stop condition: detect rising edge of SC bit 7
    if (g_stop_serial_active && !g_stop_serial_triggered) {
        static bool prev_sc_high = false;
        unsigned char sc = g_gb->externalRead(0xFF02);
        bool sc_high = (sc & 0x80) != 0;
        if (sc_high && !prev_sc_high) {
            unsigned char sb = g_gb->externalRead(0xFF01);
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
    unsigned short addr = 0;
    unsigned char value = 0;
    bool negate = false;
    bool active = false;
};

static StopCondition parse_stop_when(const std::string &spec) {
    // Format: ADDR=VAL or ADDR!=VAL (hex), e.g. A000=80 or A000!=80
    auto neq = spec.find("!=");
    auto eq = spec.find('=');
    if (eq == std::string::npos) {
        std::fprintf(stderr, "Error: --stop-when format is ADDR=VAL or ADDR!=VAL (e.g. A000!=80)\n");
        std::exit(1);
    }
    StopCondition cond;
    bool is_negate = (neq != std::string::npos && neq < eq);
    cond.addr = static_cast<unsigned short>(
        std::strtoul(spec.substr(0, is_negate ? neq : eq).c_str(), nullptr, 16));
    cond.value = static_cast<unsigned char>(
        std::strtoul(spec.substr(eq + 1).c_str(), nullptr, 16));
    cond.negate = is_negate;
    cond.active = true;
    return cond;
}

// --- Main ---

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
        "  --stop-on-serial <B> Stop when byte B (hex) is sent via serial (e.g. 0A for newline)\n"
        "  --stop-serial-count <N> Stop on Nth occurrence of serial byte (default: 1)\n"
        "  --model <model>      dmg or cgb (default: dmg)\n"
        "  --report-audio       print AUDIO=0/1 (last-frame activity) for _outaudio tests\n"
        "  --boot-rom <path>    Boot ROM file (default: skip boot)\n",
        argv0);
}

int main(int argc, char *argv[]) {
    std::string rom_path;
    std::string profile_path;
    std::string output_path;
    std::string boot_rom_path;
    int max_frames = 3000;
    long until_tcycle = -1;  // >=0: run exactly N T-cycles, capture final screen
    std::string model = "DMG-B";
    std::string reference_path;
    int extra_frames = 0;
    bool report_audio = false;
    int stop_opcode = -1;  // -1 = disabled
    unsigned load_flags = gambatte::GB::LoadFlag::NO_BIOS;
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
            if (m == "cgb" || m == "CGB") {
                // gambatte-speedrun emulates cgb04c (CPU-CGB-C); label it
                // accordingly so it lines up with missingno's CGB-C traces.
                model = "CGB-C";
                load_flags |= gambatte::GB::LoadFlag::CGB_MODE;
            }
        } else if (arg == "--reference" && i + 1 < argc) {
            reference_path = argv[++i];
        } else if (arg == "--extra-frames" && i + 1 < argc) {
            extra_frames = std::atoi(argv[++i]);
        } else if (arg == "--stop-opcode" && i + 1 < argc) {
            stop_opcode = static_cast<int>(std::strtoul(argv[++i], nullptr, 16));
        } else if (arg == "--report-audio") {
            report_audio = true;
        } else if (arg == "--help" || arg == "-h") {
            print_usage(argv[0]);
            return 0;
        }
    }

    // If a boot ROM is provided, don't skip BIOS
    if (!boot_rom_path.empty()) {
        load_flags &= ~gambatte::GB::LoadFlag::NO_BIOS;
    }

    if (rom_path.empty() || profile_path.empty() || output_path.empty()) {
        print_usage(argv[0]);
        return 1;
    }

    // Load profile
    g_profile = load_profile(profile_path);
    build_emitters(g_profile);

    std::fprintf(stderr, "Profile: %s (%zu fields)\n",
                 g_profile.name.c_str(), g_profile.fields.size());

    // Init emulator
    gambatte::GB gb;
    g_gb = &gb;

    int load_result = gb.load(rom_path, load_flags);
    if (load_result != 0) {
        std::fprintf(stderr, "Error: failed to load ROM '%s' (error %d)\n",
                     rom_path.c_str(), load_result);
        return 1;
    }

    // Load boot ROM if provided
    std::string boot_rom_info = "skip";
    if (!boot_rom_path.empty()) {
        int bios_result = gb.loadBios(boot_rom_path);
        if (bios_result != 0) {
            std::fprintf(stderr, "Error: failed to load boot ROM '%s' (error %d)\n",
                         boot_rom_path.c_str(), bios_result);
            return 1;
        }
        boot_rom_info = sha256_file(boot_rom_path);
        std::fprintf(stderr, "Boot ROM: %s (sha256: %s)\n",
                     boot_rom_path.c_str(), boot_rom_info.c_str());
    }

    // CGB output is colour → store the pix field as RGB555.
    g_cgb = (load_flags & gambatte::GB::LoadFlag::CGB_MODE) != 0;

    // Init FFI writer
    std::string rom_hash = sha256_file(rom_path);

    // Build header JSON for the FFI writer
    std::string pix_format = g_cgb ? "\"pix_format\":\"rgb555\"," : "";
    std::string header_json = "{\"_header\":true,\"format_version\":\"0.1.0\","
        "\"emulator\":\"gambatte-speedrun\",\"emulator_version\":\"r730+\","
        "\"rom_sha256\":\"" + rom_hash + "\",\"model\":\"" + model + "\","
        "\"boot_rom\":\"" + boot_rom_info + "\",\"profile\":\"" + g_profile.name + "\","
        + pix_format + "\"fields\":[";
    for (size_t i = 0; i < g_emitters.size(); i++) {
        if (i > 0) header_json += ",";
        header_json += "\"" + g_emitters[i].name + "\"";
    }
    header_json += "],\"trigger\":\"instruction\"}";

    g_writer = gbtrace_writer_new(
        output_path.c_str(), header_json.c_str(), header_json.size());
    if (!g_writer) {
        std::fprintf(stderr, "Error: failed to create trace writer\n");
        return 1;
    }

    // Cache column indices
    g_writer_cols.resize(g_emitters.size());
    for (size_t i = 0; i < g_emitters.size(); i++) {
        g_writer_cols[i] = gbtrace_writer_find_field(
            g_writer, g_emitters[i].name.c_str());
    }

    // Mark entry 0 as a frame boundary so the pre-vblank period is included
    gbtrace_writer_mark_frame(g_writer);

    std::fprintf(stderr, "Output: gbtrace (native format)\n");

    gb.setTraceCallback(trace_callback);

    // Run
    static const std::size_t SAMPLES_PER_FRAME = 35112;
    std::vector<gambatte::uint_least32_t> video_buf(160 * 144, 0);
    std::vector<gambatte::uint_least32_t> audio_buf(SAMPLES_PER_FRAME * 2 + 2064, 0);

    for (const auto &cond : stop_conditions) {
        std::fprintf(stderr, "Stop condition: [0x%04X] == 0x%02X\n",
                     cond.addr, cond.value);
    }
    if (g_stop_serial_active) {
        std::fprintf(stderr, "Stop on serial byte: 0x%02X (after %d occurrence%s)\n",
                     g_stop_serial_byte, g_stop_serial_count,
                     g_stop_serial_count == 1 ? "" : "s");
    }

    // Set up opcode stop
    g_stop_opcode = stop_opcode;
    if (stop_opcode >= 0) {
        std::fprintf(stderr, "Stop on opcode: 0x%02X\n", stop_opcode);
    }

    // Set up pixel capture
    g_video_buf_ptr = video_buf.data();

    // Load reference image
    bool has_reference = false;
    if (!reference_path.empty()) {
        if (load_reference(reference_path)) {
            has_reference = true;
            std::fprintf(stderr, "Reference: %s (%d pixels)\n",
                         reference_path.c_str(), 160 * 144);
        } else {
            std::fprintf(stderr, "Warning: could not load reference '%s'\n",
                         reference_path.c_str());
        }
    }

    int frames = 0;
    bool stopped_early = false;
    int remaining_extra = -1;  // -1 = not triggered yet
    std::size_t last_audio_samples = 0;

    // Cycle-budget mode (gambatte tests): run for exactly N T-cycles and capture
    // the screen at that instant, matching gambatte's own testrunner / missingno
    // (which read the screen after a fixed cycle budget rather than counting
    // vblank events). Frame-event counting samples the wrong screen for tests
    // that disable the display or stall the CPU, where vblanks and cycles
    // diverge. gambatte samples audio at 2097152 Hz (half the 4194304 Hz CPU
    // clock), so one sample == two T-cycles — 35112 samples == one 70224-T-cycle
    // frame.
    if (until_tcycle >= 0) {
        // Replicate gambatte's own testrunner loop exactly (test/testrunner.cpp):
        // run samples_per_frame (35112) chunks while samplesLeft >= 0, then read
        // the framebuffer. One 35112-sample chunk == one 70224-T-cycle frame.
        // This is what produced the `_out<hex>` reference values, so gambatte
        // matches them by construction; matching the chunked loop (not an exact
        // cycle count) is what gets the right completed frame for display-toggling
        // tests, where vblank-counting would sample the wrong screen.
        long samples_left = (long)SAMPLES_PER_FRAME * (until_tcycle / 70224);
        while (samples_left >= 0) {
            std::size_t samples = SAMPLES_PER_FRAME;
            gb.runFor(video_buf.data(), 160, audio_buf.data(), samples);
            samples_left -= (long)samples;
            last_audio_samples = samples;
        }
        // video_buf holds the final screen. Emit it as the trace's last frame:
        // capture pixels, mark the boundary, then run a few samples so the
        // per-instruction trace callback flushes the pending pix into an entry
        // (render reconstructs a frame from the entries after its marker).
        if (g_has_pix || has_reference) capture_frame_pixels();
        gbtrace_writer_mark_frame(g_writer);
        frames++;
        // Emit the captured screen as a final trace entry directly, rather than
        // running more instructions to trigger the per-instruction callback —
        // halt/blank tests leave the CPU halted, so no further callback fires.
        // CPU-register columns are zeroed (max cb_index is 10); only the pix
        // matters for the screenshot/blank check, and IO columns read live state.
        {
            int zero_regs[16] = {0};
            emit_entry(zero_regs);
        }
        goto run_done;
    }

    while (frames < max_frames) {
        std::size_t samples = SAMPLES_PER_FRAME;
        std::ptrdiff_t result = gb.runFor(
            video_buf.data(), 160,
            audio_buf.data(), samples);
        if (result >= 0) {
            frames++;
            last_audio_samples = samples;
            if (g_has_pix || has_reference) {
                capture_frame_pixels();
            }
            gbtrace_writer_mark_frame(g_writer);

            // Check reference match (always immediate stop — the frame we want is captured)
            if (has_reference && frame_matches_reference()) {
                std::fprintf(stderr, "Reference match at frame %d\n", frames);
                std::size_t s2 = SAMPLES_PER_FRAME;
                gb.runFor(video_buf.data(), 160, audio_buf.data(), s2);
                stopped_early = true;
                break;
            }

            // If we're in extra-frames countdown, just decrement
            if (remaining_extra >= 0) {
                if (remaining_extra == 0) {
                    stopped_early = true;
                    break;
                }
                remaining_extra--;
                continue;
            }

            // Check stop conditions — start countdown instead of breaking
            for (const auto &cond : stop_conditions) {
                bool match = gb.externalRead(cond.addr) == cond.value;
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

run_done:
    gbtrace_writer_close(g_writer);
    g_writer = nullptr;

    if (report_audio) {
        bool has_audio = last_frame_has_audio(audio_buf.data(), last_audio_samples);
        std::fprintf(stderr, "AUDIO=%d\n", has_audio ? 1 : 0);
    }

    if (stopped_early) {
        std::fprintf(stderr, "Stop condition met at frame %d, output written.\n", frames);
    } else {
        std::fprintf(stderr, "Traced %d frames, output written.\n", frames);
    }
    return 0;
}
