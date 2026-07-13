/* morepork FFI — C bindings for the morepork profile loader and trace writer.
 *
 * Link with: -lmorepork_ffi -lm -ldl -lpthread
 *
 * Usage:
 *   1. Load a profile with morepork_profile_load()
 *   2. Build the header JSON string using profile field names
 *   3. Create a writer with morepork_writer_new()
 *   4. Look up field indices with morepork_writer_find_field()
 *   5. For each trace entry:
 *      a. Call morepork_writer_set_* for each field
 *      b. Call morepork_writer_finish_entry()
 *   6. Call morepork_writer_close() to finalize
 *   7. Free the profile with morepork_profile_free()
 */
#ifndef MOREPORK_H
#define MOREPORK_H

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- Profile ---- */

/* Opaque profile handle */
typedef struct MoreporkProfile MoreporkProfile;

/* Load a profile from a TOML file.
 * path: null-terminated file path.
 * Returns profile handle, or NULL on error. */
MoreporkProfile *morepork_profile_load(const char *path);

/* Get the profile name. */
const char *morepork_profile_name(const MoreporkProfile *p);

/* Get the profile description. */
const char *morepork_profile_description(const MoreporkProfile *p);

/* Get the trigger string (e.g. "instruction", "tcycle"). */
const char *morepork_profile_trigger(const MoreporkProfile *p);

/* Get the number of fields in the profile. */
size_t morepork_profile_num_fields(const MoreporkProfile *p);

/* Get a field name by index. Returns NULL if out of bounds. */
const char *morepork_profile_field_name(const MoreporkProfile *p, size_t index);

/* Get the number of memory address fields. */
size_t morepork_profile_num_memory(const MoreporkProfile *p);

/* Get a memory field name by index. Returns NULL if out of bounds. */
const char *morepork_profile_memory_name(const MoreporkProfile *p, size_t index);

/* Get a memory field address by index. Returns 0 if out of bounds. */
uint16_t morepork_profile_memory_addr(const MoreporkProfile *p, size_t index);

/* Free a profile handle. */
void morepork_profile_free(MoreporkProfile *p);

/* ---- Writer ---- */

/* Opaque writer handle */
typedef struct MoreporkWriter MoreporkWriter;

/* Field type constants (returned by morepork_writer_field_type) */
#define MOREPORK_TYPE_U8   0
#define MOREPORK_TYPE_U16  1
#define MOREPORK_TYPE_U64  2
#define MOREPORK_TYPE_BOOL 3
#define MOREPORK_TYPE_STR  4

/* Create a new trace writer.
 * path: null-terminated output file path.
 * header_json: pointer to JSON header string. May include a "family" key
 *   naming the console family ("gb", ...); absent means "gb". The writer
 *   fills in the self-describing metadata (field_defs, field_groups,
 *   instruction_addr_field, snapshot_kinds) itself.
 * header_len: byte length of header_json (not null-terminated).
 * Returns writer handle, or NULL on error. */
MoreporkWriter *morepork_writer_new(const char *path,
                                   const char *header_json,
                                   size_t header_len);

/* Return the number of fields in the trace. */
size_t morepork_writer_num_fields(const MoreporkWriter *w);

/* Find the column index of a field by name. Returns -1 if not found. */
int morepork_writer_find_field(const MoreporkWriter *w, const char *name);

/* Get the field type for a column index.
 * Returns MOREPORK_TYPE_* constant, or -1 if invalid. */
int morepork_writer_field_type(const MoreporkWriter *w, size_t field);

/* Set field values. Call one per field per entry. */
void morepork_writer_set_u8(MoreporkWriter *w, size_t field, uint8_t value);
void morepork_writer_set_u16(MoreporkWriter *w, size_t field, uint16_t value);
void morepork_writer_set_u64(MoreporkWriter *w, size_t field, uint64_t value);
void morepork_writer_set_bool(MoreporkWriter *w, size_t field, bool value);
void morepork_writer_set_str(MoreporkWriter *w, size_t field,
                             const char *ptr, size_t len);

/* Append a null value for a nullable field (pix, vram_addr, vram_data). */
void morepork_writer_set_null(MoreporkWriter *w, size_t field);

/* Mark a frame boundary at the current entry position.
 * Call at vblank. Writes boundary to metadata and flushes row group.
 * Returns 0 on success, -1 on error. */
int morepork_writer_mark_frame(MoreporkWriter *w);

/* Mark a frame boundary carrying an indexed-frame snapshot (palette + pixel
 * indices), for palette-indexed displays like the VCS.
 *   palette_rgb: palette_len*3 bytes (R,G,B per entry)
 *   pixels:      width*height bytes, each an index into the palette
 * Returns 0 on success, -1 on error. */
int morepork_writer_mark_frame_indexed(MoreporkWriter *w,
                                       uint16_t width, uint16_t height,
                                       float pixel_aspect,
                                       const uint8_t *palette_rgb, size_t palette_len,
                                       const uint8_t *pixels, size_t pixels_len);

/* Finish the current entry (after setting all fields).
 * Returns 0 on success, -1 on error. */
int morepork_writer_finish_entry(MoreporkWriter *w);

/* Close the writer and finalize the trace file.
 * Consumes the writer — do not use after this call.
 * Returns 0 on success, -1 on error. */
int morepork_writer_close(MoreporkWriter *w);

#ifdef __cplusplus
}
#endif

#endif /* MOREPORK_H */
