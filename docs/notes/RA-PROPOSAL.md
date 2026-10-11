# Atari 5200 for RetroAchievements: memory map and hashing

RetroAchievements lists the Atari 5200 as unsupported, needing a "hashing
method and memory map". This document proposes both, with the evidence for
each, in a form RA can adopt. AnticRust is the libretro core it was checked
against.

Status 2026-10-11: **both are done and verified.** Every check below is a
script in this repo that can be re-run, and each was shown to fail when the
thing it checks is broken.

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

## Hashing

### The problem

**rcheevos has no hash method for console 50 today.** In the version our
host ships, `src/rhash/hash.c` rejects it both ways:
- "Unsupported console for buffer hash: 50";
- "Unsupported console for file hash: 50".

Hashing the whole file, the obvious default, would be wrong for this
machine. The 215-file corpus (69 commercial, 117 homebrew and hacks, 29
prototypes) shows:

- **No file carries a header.** Every `.a52`, `.bin` and `.rom` is a raw
  image whose size is a multiple of 4096.
- **The same program circulates at several sizes:**
  - an 8K image repeated to 16K and 32K (Q-bert);
  - a 16K image repeated to 32K (Blaster);
  - a two-chip 16K image written out as a 32K `[lo, lo, hi, hi]` (Crazy
    Otto, Pac-Man, Pirate Pete, Super Pac-Man).

  A whole-file hash makes each of these a different game, so a set approved
  against one dump would not load the other.

### Proposal

Hash one canonical image:

1. **Skip an atari800 `CART` container header** (16 bytes: `CART`, type,
   checksum), when the rest is a cartridge size. None appear in the corpus,
   but the format exists. A header identifies the dumper, not the game.
2. **While the image is its first half twice, keep only the first half,**
   down to 4K.
3. **Hash a 32K image of 8K blocks `[A, A, B, B]` as `A` then `B`:** the
   two-chip 16K cartridge it was written out from.
4. **Hash anything else whole,** including the 40K Bounty Bob image.

Then take the MD5 as usual. In rcheevos terms this is `rc_hash_atari5200`
in `tools/rcheevos/rc_hash_atari5200.c`, plus four one-line additions that
`tools/rcheevos/apply_5200.py` makes:
- the prototype in `rc_hash_internal.h`;
- the buffer dispatch in `hash.c`;
- the file dispatch in `hash.c` (buffered, like the 7800);
- `{ "a52", ..., RC_CONSOLE_ATARI_5200 }` in the extension table.

### Why the rules are safe: they never merge two images that run differently

Each rule undoes a layout the hardware cannot tell apart:
- In the 32K cartridge window, an image repeated to fill it reads the same
  at every address.
- Linear `[A, A, B, B]` puts A at $4000 and $6000 and B at $8000 and $A000,
  which is exactly the two-chip mapping of `A ++ B`.

That is measured, not argued. `rahash` (`crates/antic-runner/src/rahash.rs`)
runs every file whose canonical image differs from it, original against
canonical, on the emulator, with the same scripted input for 1800 frames:

    18 of 18 changed files run identically from their canonical image

That means every frame pixel-identical and all RAM identical, at every
checkpoint. The canonical image runs under the mapping the reduction implies:
a halved 32K is one-chip, an `[A, A, B, B]` is two-chip. Swapping A and B in
rule 3 makes 8 of them diverge from the first checkpoint.

The implied mapping also agrees with the mapping measured independently. The
census runs every 16K file under both layouts and keeps the one that runs.
Wherever the hash joins a 32K file to a 16K one, they agree. For example,
`Blaster.bin` (32K, halves to 16K, so one-chip) joins the prototype
`Blaster (1984)`, which the census measured one-chip.

### What it merges

    215 files: 212 distinct file hashes, 202 distinct canonical hashes

The 10 extra merges are all the same program at two or three sizes:
- Q-bert-nobutton at 8K, 16K and 32K;
- Blaster (homebrew 32K and prototype 16K);
- the 32K and 16K copies of Crazy Otto, Pac-Man (Bugs Fixed Hack), each of
  the three Pirate Pete versions, and Super Pac-Man;
- Pete's Diagnostic Cart and the prototype Pete's Diagnostics.

The three groups that already shared a file hash were byte-identical
duplicates under different names.

### Verified in rcheevos

`tools/rcheevos/hash-check.sh` applies the proposal to the host's
`origin/main` rcheevos. It hashes all 215 files through
`rc_hash_generate_from_buffer` and through `rc_hash_generate_from_file`
(which also exercises the `a52` routing), and compares both with
AnticRust's own implementation:

    215 of 215 files: rcheevos buffer hash == rcheevos file hash == AnticRust canonical MD5

Breaking the C version's rule 3 drops this to 206 of 215.

### Licence

rcheevos is MIT; this repository is GPL. Offering `rc_hash_atari5200.c`
upstream would need it relicensed to MIT. That is the owner's decision and
has not been made.
