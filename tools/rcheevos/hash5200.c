// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000
/* Print rcheevos' Atari 5200 hash for each file, both ways rcheevos can
 * compute one: from a buffer, and from a path (which also checks the "a52"
 * extension routing). Output: <buffer hash> <file hash> <path> */
#include "rc_hash.h"
#include "rc_consoles.h"
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char** argv) {
  int bad = 0;
  for (int i = 1; i < argc; i++) {
    FILE* f = fopen(argv[i], "rb");
    if (!f) { printf("cannot open %s\n", argv[i]); bad = 1; continue; }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    unsigned char* buf = malloc(n);
    fread(buf, 1, n, f);
    fclose(f);
    char hb[33] = "", hf[33] = "";
    int ob = rc_hash_generate_from_buffer(hb, RC_CONSOLE_ATARI_5200, buf, n);
    int of = rc_hash_generate_from_file(hf, RC_CONSOLE_ATARI_5200, argv[i]);
    printf("%s %s %s\n", ob ? hb : "FAIL", of ? hf : "FAIL", argv[i]);
    free(buf);
  }
  return bad;
}
