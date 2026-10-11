/* Proposed Atari 5200 hash for rcheevos, for src/rhash/hash_rom.c.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 * Copyright (C) 2026 Prodigy75000
 *
 * Written as rcheevos would carry it, to check the rule inside rcheevos.
 * Not submitted upstream.
 *
 * The same cartridge circulates at several sizes: an image repeated to fill
 * 16K or 32K, and a two-chip 16K cartridge written out as 32K in the order
 * [lo, lo, hi, hi]. Hashing the file's bytes would call each a different
 * game. This hashes one canonical image instead, by rules that never merge
 * two images which run differently (checked against an emulator on a
 * 215-file corpus; see AnticRust docs/notes/RA-PROPOSAL.md):
 *
 *   1. an atari800 CART container header (16 bytes) is skipped;
 *   2. while the image is its first half twice, only the first half counts,
 *      down to 4K;
 *   3. a 32K image of 8K blocks [A, A, B, B] is hashed as A then B.
 *
 * Anything else, including a 40K Bounty Bob image, is hashed whole.
 */

static int rc_hash_atari5200_image_size(size_t size)
{
  return size == 0x1000 || size == 0x2000 || size == 0x4000 || size == 0x8000 || size == 0xA000;
}

int rc_hash_atari5200(char hash[33], const rc_hash_iterator_t* iterator)
{
  const uint8_t* buffer = iterator->buffer;
  size_t size = iterator->buffer_size;
  md5_state_t md5;

  /* 1. atari800's CART container: "CART", type, checksum, then the image */
  if (size > 16 && memcmp(buffer, "CART", 4) == 0 && rc_hash_atari5200_image_size(size - 16)) {
    rc_hash_iterator_verbose(iterator, "Ignoring CART header");
    buffer += 16;
    size -= 16;
  }

  /* 2. an image repeated to fill the window: hash one copy */
  while (size > 0x1000 && (size & (size - 1)) == 0 && memcmp(buffer, buffer + size / 2, size / 2) == 0) {
    size /= 2;
    rc_hash_iterator_verbose_formatted(iterator, "Image repeats, hashing the first %u bytes", (unsigned)size);
  }

  md5_init(&md5);

  /* 3. a two-chip 16K written out as [A, A, B, B]: hash A then B */
  if (size == 0x8000 &&
      memcmp(buffer, buffer + 0x2000, 0x2000) == 0 &&
      memcmp(buffer + 0x4000, buffer + 0x6000, 0x2000) == 0) {
    rc_hash_iterator_verbose(iterator, "Two-chip 16K written out as 32K, hashing its two chips");
    md5_append(&md5, buffer, 0x2000);
    md5_append(&md5, buffer + 0x4000, 0x2000);
  }
  else {
    rc_hash_iterator_verbose_formatted(iterator, "Hashing %u byte image", (unsigned)size);
    md5_append(&md5, buffer, (int)size);
  }

  return rc_hash_finalize(iterator, &md5, hash);
}
