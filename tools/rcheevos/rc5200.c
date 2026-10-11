// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000
/* Drive AnticRust's libretro DLL through rcheevos' own memory code. */
#include "rc_libretro.h"
#include "rc_consoles.h"
#include <windows.h>
#include <stdio.h>
#include <string.h>

static struct retro_memory_descriptor descs[8];
static struct retro_memory_map mmap_seen;
static void* (*get_data)(unsigned);
static size_t (*get_size)(unsigned);

static bool env(unsigned cmd, void* data) {
  if (cmd == (36 | 0x10000)) {
    const struct retro_memory_map* m = (const struct retro_memory_map*)data;
    mmap_seen.num_descriptors = m->num_descriptors;
    memcpy(descs, m->descriptors, m->num_descriptors * sizeof(descs[0]));
    mmap_seen.descriptors = descs;
  }
  return true;
}
static void video(const void* d, unsigned w, unsigned h, size_t p) { (void)d; (void)w; (void)h; (void)p; }
static size_t audio(const int16_t* d, size_t n) { (void)d; return n; }
static void poll(void) {}
static int16_t state(unsigned a, unsigned b, unsigned c, unsigned d) { (void)a; (void)b; (void)c; (void)d; return 0; }
static void core_info(uint32_t id, rc_libretro_core_memory_info_t* info) {
  info->data = (uint8_t*)get_data(id);
  info->size = get_size(id);
}

int main(int argc, char** argv) {
  HMODULE h = LoadLibraryA(argv[1]);
  if (!h) { printf("cannot load %s\n", argv[1]); return 2; }
#define SYM(n) GetProcAddress(h, n)
  ((void (*)(void*))SYM("retro_set_environment"))((void*)env);
  ((void (*)(void*))SYM("retro_set_video_refresh"))((void*)video);
  ((void (*)(void*))SYM("retro_set_audio_sample_batch"))((void*)audio);
  ((void (*)(void*))SYM("retro_set_input_poll"))((void*)poll);
  ((void (*)(void*))SYM("retro_set_input_state"))((void*)state);
  get_data = (void* (*)(unsigned))SYM("retro_get_memory_data");
  get_size = (size_t (*)(unsigned))SYM("retro_get_memory_size");
  FILE* f = fopen(argv[2], "rb");
  static unsigned char rom[65536];
  size_t n = fread(rom, 1, sizeof rom, f);
  fclose(f);
  struct game_info { const char* path; const void* data; size_t size; const char* meta; } gi = { NULL, rom, n, NULL };
  if (!((bool (*)(const void*))SYM("retro_load_game"))(&gi)) { printf("load failed\n"); return 2; }
  void (*run)(void) = (void (*)(void))SYM("retro_run");
  for (int i = 0; i < 400; i++) run();

  rc_libretro_memory_regions_t regions;
  if (!rc_libretro_memory_init(&regions, &mmap_seen, core_info, RC_CONSOLE_ATARI_5200)) {
    printf("FAIL: rc_libretro_memory_init found no memory (\"waiting for core memory map\")\n");
    return 1;
  }
  const uint8_t* ram = (const uint8_t*)get_data(2);
  printf("regions %u, total size 0x%zx\n", regions.count, regions.total_size);
  int bad = 0;
  for (uint32_t a = 0; a < 0x4000; a++) {
    uint8_t* p = rc_libretro_memory_find(&regions, a);
    if (p != ram + a) { if (bad < 4) printf("  address $%04X maps to %p, RAM is %p\n", a, (void*)p, (void*)(ram + a)); bad++; }
  }
  printf("RA address == 6502 address for all 16K: %s (%d mismatches)\n", bad ? "NO" : "yes", bad);
  printf("$4000 (beyond RAM) maps to: %p\n", (void*)rc_libretro_memory_find(&regions, 0x4000));
  printf("frame counter via rcheevos: $02 = %u (direct %u)\n", *rc_libretro_memory_find(&regions, 2), ram[2]);
  return bad ? 1 : 0;
}
