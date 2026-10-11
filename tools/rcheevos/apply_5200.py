# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Prodigy75000
#
# Apply the Atari 5200 proposal (docs/notes/RA-PROPOSAL.md) to an rcheevos
# source tree: the memory region table, and the hash.
#
#   python apply_5200.py <rcheevos dir> [regions|hash|all]
#
# Each edit asserts the anchor it relies on is present exactly once and that
# the edit is not already there, so a moved upstream fails loudly instead of
# patching the wrong place.
import os
import sys

root, what = sys.argv[1], (sys.argv[2] if len(sys.argv) > 2 else "all")
here = os.path.dirname(os.path.abspath(__file__))


def edit(rel, anchor, new, before=False):
    p = os.path.join(root, rel)
    s = open(p, encoding="utf-8").read()
    assert s.count(anchor) == 1, f"{rel}: anchor found {s.count(anchor)} times: {anchor[:60]!r}"
    result = new + anchor if before else anchor + new
    assert result not in s, f"{rel}: already applied"
    s = s.replace(anchor, result, 1)
    open(p, "w", encoding="utf-8").write(s)


if what in ("regions", "all"):
    edit("src/rcheevos/consoleinfo.c", "/* ===== Atari 7800 ===== */",
         "/* ===== Atari 5200 ===== */\n"
         "/* 16K of RAM at $0000-$3FFF, published at the 6502's own addresses. */\n"
         "static const rc_memory_region_t _rc_memory_regions_atari5200[] = {\n"
         "    { 0x000000U, 0x003FFFU, 0x000000U, RC_MEMORY_TYPE_SYSTEM_RAM, \"System RAM\" }\n"
         "};\n"
         "static const rc_memory_regions_t rc_memory_regions_atari5200 = { _rc_memory_regions_atari5200, 1 };\n\n",
         before=True)
    edit("src/rcheevos/consoleinfo.c", "    case RC_CONSOLE_ATARI_7800:\n      return &rc_memory_regions_atari7800;",
         "    case RC_CONSOLE_ATARI_5200:\n      return &rc_memory_regions_atari5200;\n\n", before=True)

if what in ("hash", "all"):
    body = open(os.path.join(here, "rc_hash_atari5200.c"), encoding="utf-8").read()
    p = os.path.join(root, "src/rhash/hash_rom.c")
    s = open(p, encoding="utf-8").read()
    assert "rc_hash_atari5200" not in s, "hash_rom.c: already applied"
    open(p, "w", encoding="utf-8").write(s + "\n" + body)
    edit("src/rhash/rc_hash_internal.h", "  int rc_hash_7800(char hash[33], const rc_hash_iterator_t* iterator);",
         "  int rc_hash_atari5200(char hash[33], const rc_hash_iterator_t* iterator);\n", before=True)
    # Buffer dispatch.
    edit("src/rhash/hash.c", "    case RC_CONSOLE_ATARI_7800:\n      return rc_hash_7800(hash, iterator);",
         "    case RC_CONSOLE_ATARI_5200:\n      return rc_hash_atari5200(hash, iterator);\n\n", before=True)
    # File dispatch: buffer the file, then the buffer path above.
    edit("src/rhash/hash.c", "    case RC_CONSOLE_ATARI_7800:\n    case RC_CONSOLE_ATARI_LYNX:\n",
         "    case RC_CONSOLE_ATARI_5200:\n", before=True)
    # Extension.
    edit("src/rhash/hash.c", '  { "a78", rc_hash_initialize_iterator_single, RC_CONSOLE_ATARI_7800 },',
         '  { "a52", rc_hash_initialize_iterator_single, RC_CONSOLE_ATARI_5200 },\n', before=True)

print(f"applied {what} to {root}")
