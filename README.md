<!--
SPDX-License-Identifier: GPL-3.0-or-later
Copyright (C) 2026 Prodigy75000
-->

# AnticRust

A clean-room Atari 5200 emulator core written from scratch in Rust. No C, no
bindings, and no third-party emulator code.

Named for ANTIC, the display-list processor the 5200 shares with every Atari
8-bit machine, so the name still fits if the 400 and 800 are ever folded in.

Started 2026-10-11. Nothing works yet.

## This core has two objectives, and the second one is why it exists

**1. No BIOS.** The HLE boots a cartridge with no firmware dump present. Same
objective as ColecoRust and IntelliRust, and the same reason: needing a BIOS
file has always been a drag, and it is worst on machines whose games are
shorter than their setup.

**2. RetroAchievements support, which nothing currently provides.** This is the
gap that prompted the project. RA lists the Atari 5200 as unsupported, and the
reason is not that nobody tried:

> **Atari 5200** - Not supported, needs hashing method and memory map.
>
> - libretro core `atari800`: appears to work fine, but is a headache to use.
>   The BIOS has to be configured inside the core. Controls are unclear.
>   **It allows loading arbitrary files from its internal menu, which bypasses
>   hashing.**
> - libretro core `a5200`: **does not expose memory.**

Every one of those is a design decision rather than a bug, and each has an
obvious opposite. So the bar here is not "emulate the 5200"; it is "be the core
an achievement set can be approved against".

## What that means concretely, as constraints rather than aspirations

- **Publish `SET_MEMORY_MAPS` and `SYSTEM_RAM`.** A core that omits them leaves
  RetroAchievements hanging on "waiting for core memory map". This is already a
  standing rule across every in-house core here; on this one it is the point.
- **No in-core file browser, ever.** Content arrives from the frontend and
  nowhere else. `atari800` is unusable for achievements precisely because its
  internal menu can load a file the frontend never hashed, so the hash RA
  checked and the bytes actually running can differ. A core with no way to load
  its own content cannot have that bug.
- **No BIOS configuration UI.** There is nothing to configure, because there is
  no BIOS. The HLE objective and the RA objective happen to want the same
  thing here.
- **A stable hashing method.** RA needs to identify a cartridge the same way
  every time. Deciding what the hash covers (and what it must exclude, such as
  headers a dumper may or may not have added) is work this core owes, not work
  it inherits.

## The machine

| Part | What it is |
|---|---|
| CPU | 6502C "SALLY" at 1.79 MHz |
| Display | ANTIC, a display-list processor that fetches its own instructions, plus GTIA for the pixels |
| Sound and input | POKEY, four channels, and it reads the keypad and the analog pots |
| RAM | 16 KB |
| BIOS | 2 KB |
| Cartridge | commonly 8 KB to 32 KB, with bank switching above that |
| Controllers | four ports; analog joysticks that do NOT self-centre, a 12-key keypad, and Start, Pause and Reset |

Architecturally this is an Atari 400/800 in a console shell, which is why the
chipset name fits the repo and why Atari 8-bit documentation applies directly.

## How correctness gets decided

- **A real BIOS is the oracle, never a requirement.** Any dump in `bios/` is a
  development tool. Run a title both ways and diff. When HLE and the real BIOS
  disagree, the real one is right until proven otherwise. A core that quietly
  starts needing the dump has failed at its first objective.
- **A verdict is a ledger row, not a memory.** Per-title, recorded, with a fixed
  vocabulary from the start and the verdict derived rather than typed.
- **Tests must be able to fail.** Prove a new test fails before trusting it.
- **"Majority of commercial titles" needs a denominator.** State coverage as a
  fraction of a named corpus.

## Layout

    crates/antic-core       the machine: 6502C, ANTIC, GTIA, POKEY, mapper, HLE BIOS
    crates/antic-libretro   the shipped cdylib: the C ABI, memory maps, hashing
    crates/antic-runner     dev harnesses: test ROMs, headless renders, HLE-vs-BIOS diffs
    docs/notes              our own design notes and measurements
    docs/ref                derived reference write-ups that cite their sources

## House rules inherited from the other in-house cores

- **Save states are a byte-identical contract.** Fixed size, little-endian,
  versioned, identical on x86-64 and arm64.
- **No third-party documents in git.** Only markdown under `docs/`, and only our
  own notes plus derived write-ups that cite their sources.
- **Cartridge images and BIOS dumps never enter the repository.**

## Status

| Component | State |
|-----------|-------|
| 6502C SALLY | not started |
| ANTIC | not started |
| GTIA | not started |
| POKEY (sound) | not started |
| POKEY (keypad and pots) | not started |
| Analog controllers | not started |
| Mapper / bank switching | not started |
| HLE BIOS | not started |
| Save states | not started |
| libretro, memory maps, hashing | not started |
