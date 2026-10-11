# bios/

**You should not need anything in here.** AnticRust boots a cartridge on its own
HLE BIOS, with no firmware dump present. That is the first of the project's two
objectives rather than a nice-to-have, and the core has no way to load one.

What this directory is for is DEVELOPMENT. A real 2 KB BIOS is the oracle: when
the HLE and the hardware disagree about what a game sees, the fastest way to
find out which is wrong is to run both and diff.

Nothing here is tracked. The BIOS is copyrighted code and can never be
committed, in any revision.

Note that RetroAchievements specifically faults the existing `atari800` core for
requiring its BIOS to be configured inside the core. A core with no BIOS has no
such surface, which is one of the places the two objectives agree.
