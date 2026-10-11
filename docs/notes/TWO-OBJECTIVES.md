# Two objectives: no BIOS, and be approvable by RetroAchievements

Written 2026-10-11, before any emulation existed, when the repo was set up.
Everything here is either the owner's stated objective or a question flagged
open. Nothing here is a measurement, because there is nothing yet to measure.

## Objective 1: no BIOS

Run the majority of commercial titles with no firmware dump present. Same as
ColecoRust and IntelliRust, same reason: needing a BIOS file has always been a
drag and it is worst on machines whose games are shorter than their setup. The
owner is explicit that no further BIOS machinery is going into the app, and
that the existing machinery is being torn down rather than extended.

A real BIOS is the ORACLE, never a requirement. Run a title both ways and diff;
when they disagree the real one is right until proven otherwise.

## Objective 2: be the core an achievement set can be approved against

This is the gap that prompted the project, and it is unusual in being a
*product* gap rather than a technical one. RetroAchievements lists the Atari
5200 as unsupported, needing "hashing method and memory map". Two libretro
cores exist and each fails for a different reason:

**`a5200`: does not expose memory.** Nothing else about it matters for this
purpose. Without a memory map, RA has nothing to watch, and a frontend sits on
"waiting for core memory map".

**`atari800`: allows loading arbitrary files from its internal menu, which
bypasses hashing.** This is the subtler and more interesting failure. RA
identifies a game by hashing what the frontend loaded. If the core can open a
file by itself, the bytes executing are no longer necessarily the bytes that
were hashed, so no set can be trusted. RA also notes the BIOS has to be
configured inside the core, and that the controls are hard to work out.

Each of those has an obvious opposite, and they are constraints on this
repository rather than aspirations:

| Their failure | Our constraint |
|---|---|
| no memory exposed | publish `SET_MEMORY_MAPS` and `SYSTEM_RAM` |
| internal file browser bypasses hashing | the core has NO content file IO at all |
| BIOS configured in-core | there is no BIOS, so nothing to configure |
| controls unclear | input descriptors that say what the controls are |

`antic_core::ContentSource` is a single-variant enum with a test asserting it
stays that way. That is not ceremony: it means adding a second way for content
to reach the core is a visible argument in a diff rather than a quiet
convenience, because the second way is exactly what makes a core unusable for
achievements.

The two objectives happen to agree. An HLE core with no firmware has nothing to
configure, and a core that cannot open files cannot be made to run unhashed
bytes.

## What the hashing method has to decide

RA needs to identify a cartridge the same way every time, and this core owes
that decision rather than inheriting it. The question is what the hash covers.
Dumps in the wild may or may not carry a header; a hash that includes one
identifies the dumper rather than the game, and the same cartridge from two
sources then looks like two games. The answer should be written down with its
reasoning once there is a corpus to check it against, and it should be stated
in a way RA can adopt rather than merely implemented.

## The machine, and two things that catch a 6502 author out

6502C "SALLY" at 1.79 MHz, ANTIC and GTIA for display, POKEY for sound and for
reading the keypad and pots, 16 KB RAM, a 2 KB BIOS. Architecturally an Atari
400/800 in a console shell, which is why Atari 8-bit documentation applies
directly and why the repo is named for the chipset rather than the console.

**ANTIC is not a video chip you poke.** It is a coprocessor that fetches and
executes its own display list out of RAM, steals cycles from the CPU to do it,
and drives GTIA. The CPU and ANTIC share a bus and the contention is
observable, so timing is not a layer to add afterwards.

**The joysticks are analog and do not self-centre.** They are read as pot values
through POKEY rather than as switch closures. A digital stick mapped to the
extremes will feel wrong and be wrong, and "centre" is a value the hardware
never reliably returns on its own. This is probably also why RA could not work
out `atari800`'s controls.

## Open questions, deliberately not decided here

**Where the HLE intercepts.** ColecoRust measured this rather than reasoning
about it and found the answer was BOTH trap-at-PC and a synthesised image,
because 71 of its 164 titles read firmware bytes as DATA. The equivalent number
here is unknown and is a census question.

**What the hash covers.** See above. Needs the corpus.

**Mapper coverage.** Which bank-switching schemes actually appear in the corpus,
as opposed to which ones exist.

**Whether the boot sequence is emulated, skipped or configurable.** The owner
decided SKIPPED for ColecoVision. Do not assume it transfers. IntelliRust found
that on its machine "skip" really means "synthesise the post-boot state",
because the firmware does setup the game depends on, and separated the visible
title screen from the work behind it. Measure that boundary here before asking.

**Settled 2026-10-11: SKIPPED.** The census measured the boundary (4.07
frames of setup, 254.89 frames of logo; see BIOS-CENSUS.md), and the owner
confirmed skip. The HLE synthesises the post-boot state and shows no logo;
there is no option to show one (HLE.md).

## The method that is worth copying from the siblings

ColecoRust did not start on the HLE. It built an instrument first: a CPU green
against a suite that prints its own verdict, then the least bus that runs the
REAL firmware, then a census over the whole corpus recording every control
transfer into the firmware region, every DATA read of it by cartridge code, and
which RAM bytes the firmware writes that the cartridge later reads.

That turned judgement calls into measurements, and re-running the census in HLE
mode became the diff harness, which is the oracle rule expressed as code. None
of that code transfers to a 6502 with ANTIC. The order of operations does.
