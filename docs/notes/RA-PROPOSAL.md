# Atari 5200 for RetroAchievements: memory map and hashing

RetroAchievements lists the Atari 5200 as unsupported, needing a "hashing
method and memory map". This document proposes both, with the evidence for
each, in a form RA can adopt. AnticRust is the libretro core it was checked
against.

Status 2026-10-11: **the memory map is done and verified. Hashing is still
open** (see the end).

## Memory map

### Proposal

The 5200 has 16K of RAM at $0000-$3FFF, and nothing else a set needs to
read. Publish it at the 6502's own addresses, so that an address in a set is
the address in a disassembly. As an rcheevos region table
(`src/rcheevos/consoleinfo.c`):

```c
/* ===== Atari 5200 ===== */
/* 16K of RAM at $0000-$3FFF, published at the 6502's own addresses. */
static const rc_memory_region_t _rc_memory_regions_atari5200[] = {
    { 0x000000U, 0x003FFFU, 0x000000U, RC_MEMORY_TYPE_SYSTEM_RAM, "System RAM" }
};
static const rc_memory_regions_t rc_memory_regions_atari5200 = { _rc_memory_regions_atari5200, 1 };
```

with `case RC_CONSOLE_ATARI_5200: return &rc_memory_regions_atari5200;` in
`rc_console_memory_regions`.

### Why only RAM

- **The cartridge window ($4000-$BFFF) is left out on purpose.** Bounty Bob
  Strikes Back switches two 4K banks inside it. A region over a switchable
  bank reads the right byte only until the game next switches. A set that
  needs to identify cartridge code has the hash for that.
- **The chips ($C000 GTIA, $D400 ANTIC, $E800 POKEY) read back different
  things from what was written,** and reading some of them has side effects.
  Nothing a set should watch lives there.
- **The BIOS ($F800-$FFFF) is a fixed 2K.** It has no game state.

### What the core publishes

AnticRust exposes the RAM both ways a frontend can ask for it:

- `retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM)`: 0x4000 bytes, the
  machine's live RAM, not a copy.
- `SET_MEMORY_MAPS` (`36 | 0x10000`): one descriptor with flags
  `RETRO_MEMDESC_SYSTEM_RAM`, start 0, select 0, length 0x4000, pointing at
  the same bytes.

It also sends `SET_SUPPORT_ACHIEVEMENTS` (`42 | 0x10000`). It re-sends the
map after `retro_reset`, because a reset rebuilds the machine and the RAM
moves.

### Verified through rcheevos itself

`tools/rcheevos/check.sh <cart>` builds rcheevos from the Trophy Hub host's
`origin/main` twice and drives the real core DLL through
`rc_libretro_memory_init` for console 50:

- **Stock rcheevos** has no Atari 5200 table. It ignores memory maps and lays
  out SYSTEM_RAM from address 0. This is what ships today.
- **With the table above added,** it builds its regions from
  `SET_MEMORY_MAPS` instead.

Results on Pac-Man after 400 frames, and on River Raid, under both builds:

    regions 1, total size 0x4000
    RA address == 6502 address for all 16K: yes (0 mismatches)
    $4000 (beyond RAM) maps to: 0000000000000000
    frame counter via rcheevos: $02 = 143 (direct 143)

The two paths agree, so **a set written against today's rcheevos keeps
every address when the table lands.**

The check can fail. Shifting the published descriptor's start by $100 makes
`rc_libretro_memory_init` find no memory at all. That is the "waiting for
core memory map" hang that `a5200` produces.

### Notes for set developers

These are observations from running the 69 commercial titles
(`BIOS-CENSUS.md`), not guarantees for any one game:

- **$0100-$01FF is the 6502 stack.**
- **$0200-$021F holds the interrupt vectors.**
- **Titles that keep the BIOS's vertical-blank handler** (42 of 69) have a
  frame counter at $02, incremented every frame, with $01 and $04 above it.
  They also have stick positions at $11-$18, two per controller, 1 to 227
  with no fixed centre.
- **The rest of zero page and RAM is the game's own.**

## Hashing (open)

What is settled, from `HASHING-CORPUS-CENSUS.md` and the cartridge census:

- **No file in the 215-file corpus carries a header.** Every `.a52`, `.bin`
  and `.rom` file is a raw image whose size is a multiple of 4096. The corpus
  holds no `.a52` and `.bin` copy of the same cartridge, so headered dumps
  elsewhere cannot be ruled out. If one exists, its size would show it.
- **The real hazard is layout.** The same program appears at several sizes:
  - mirrored: an 8K image repeated to 16K or 32K;
  - a two-chip 16K image written out as a 32K `[lo, lo, hi, hi]`.

  A hash over file bytes calls each of these a different game.
- **The size does not decide the mapping.** Of 40 commercial 16K images, 25
  are two-chip and 15 one-chip, measured by running each both ways.

A rule (hash the smallest image the layout reduces to) is drafted in
`HASHING-CORPUS-CENSUS.md`. It still has to be checked against every mapping
before it is proposed here.
