<!--
SPDX-License-Identifier: GPL-3.0-or-later
Copyright (C) 2026 Prodigy75000
-->

# AnticRust

An Atari 5200 emulator core written from scratch in Rust. No C and no bindings.
It ships as a [libretro](https://www.libretro.com/) core, and **it needs no
BIOS**: it boots cartridges on its own high-level emulation of the 2 KB
firmware, so there is no `5200.rom` to find and nothing to configure.

Named for ANTIC, the display-list processor the 5200 shares with the Atari 8-bit
computers.

## Status

| Component | State |
|-----------|-------|
| CPU (6502C "SALLY") | ✅ cycle-stepped, every read and write is one bus cycle; NMOS with decimal mode; passes Klaus Dormann's functional test and the SingleStepTests 6502 vectors |
| ANTIC | ✅ display lists, every playfield mode, per-line DMA slot timing (missiles, players, display list, playfield, refresh), WSYNC, VBI and DLI NMIs, fine scrolling |
| GTIA | ✅ playfield, players and missiles, priority, collisions, rendered per colour clock and caught up before every register write |
| POKEY: sound | ✅ four channels, 4-, 5-, 9- and 17-bit polynomial noise, high-pass filters, 16-bit linked channels, 48 kHz output |
| POKEY: input | ✅ analog pots, keypad scan, timers and IRQs |
| Controllers | ✅ analog stick (it does not self-centre), both fire buttons, the 12-key keypad, Start, Pause and Reset, on four ports |
| Cartridges | ✅ 4K and 8K (mirrored), 16K one-chip and two-chip, 32K, and Bounty Bob Strikes Back's 40K bank switching |
| HLE BIOS | ✅ a 2 KB image built in Rust at the documented entry points; see [HLE BIOS](#hle-bios) |
| Save states | ✅ fixed size (16,907 bytes), the same bytes on every platform; a state restores exactly |
| Memory map | ✅ `SYSTEM_RAM` and `SET_MEMORY_MAPS` for the 16 KB of RAM, for RetroAchievements, cheats and RAM watch |

Compatibility, measured over a 215-file collection (69 commercial, 29 prototype,
117 homebrew and hacks):

- **68 of 69** commercial titles run identically on the HLE BIOS and on the
  real BIOS from the moment the cartridge takes over: same frames and same RAM
  at six checkpoints up to 30 seconds. The one exception, *PAM SALT*, is
  Atari's diagnostic cartridge and checksums the BIOS itself.
- **29 of 29** prototypes and **105 of 117** homebrew titles do the same. The
  12 homebrew titles that differ all copy data out of the real BIOS's boot
  screen.
- **214 of 214** loadable files restore a save state exactly: every frame and
  every byte of RAM match for 300 frames after the load. The 215th is a 512K
  homebrew cartridge with a mapping the core does not implement.

## HLE BIOS

The 5200's BIOS sets up the machine, shows the Atari logo, then jumps to the
cartridge. Games also call into it afterwards: its interrupt dispatchers, its
keypad handler, and a handful of fixed addresses and tables some titles read
directly. AnticRust replaces it with its own 2 KB image, assembled in Rust at
startup:

- **The documented entry points are at their real addresses**, with the
  hardware's instruction layout kept where a game's timing depends on it.
  *Pitfall!*, *Mountain King* and *K-Razy Shoot-Out* copy the interrupt vector
  table from `$FE95`, and it is there.
- **The boot is skipped.** The core synthesises the machine state the real BIOS
  leaves when it hands over (registers, shadow registers, display list), so a
  game starts at once. There is no logo, because drawing one would mean
  reproducing the firmware's own screen.
- **The system font matches the original**, since 45 titles display it and 24
  copy glyphs out of it.

A real BIOS is used only as a development oracle. `hlediff` runs a title on
both and compares them.

## Video and sound

The core draws the full 384x240 frame (the wide playfield) and shows the
frontend 320x224 of it (the normal playfield width), the same window as the
a5200 core. Games park unused sprites just outside it. The aspect ratio uses the
NTSC pixel shape.

The palette is a5200's NTSC table, taken as data so that games look the same
as on the core people compare against. Sound is POKEY mixed to mono at 48 kHz.

## Controls

| 5200 | RetroPad |
|------|----------|
| Stick | left analog (proportional); D-pad (full deflection) |
| Bottom fire | B or A |
| Top fire | Y or X |
| Start, Pause, Reset | START, SELECT, L |
| Keypad 1-4, 6-9 | right analog, the eight directions of the grid around 5 |
| Keypad 5, 0 | R3, L3 |
| Keypad `*`, `#` | L2, R2 |

A keyboard on port 1 also types digits, `*` and `#`.

Each 5200 controller has its own keypad, and two-player games such as
*RealSports Football* read player 2's from controller 2. Until a second pad
sends anything, the core shows player 1's keypad and top buttons on every
controller, so these games can be set up with one pad. Once a second player
touches a pad, each controller has its own keypad again, as on hardware.

## RetroAchievements

There is no achievement support for the Atari 5200 yet; RA lists it as needing a
hashing method and a memory map. This core supplies both pieces on its side:

- **Memory.** RAM is published as `SYSTEM_RAM` and as one `SET_MEMORY_MAPS`
  descriptor for `$0000-$3FFF`. With no region table for the console, rcheevos
  lays `SYSTEM_RAM` out from 0, so RA address N is 6502 address `$N`, and the
  memory map agrees with that if a table is added. `tools/rcheevos/check.sh`
  verifies this through rcheevos itself.
- **Content only from the frontend.** The core has no file browser and no way
  to open a file, so the bytes running are always the bytes the frontend
  hashed.
- **A hash proposal.** The same cartridge circulates at several sizes: repeated
  to fill 16K or 32K, or a two-chip 16K written out as 32K. The proposed hash
  is taken over a canonical image with those repeats removed.
  [`docs/notes/RA-PROPOSAL.md`](docs/notes/RA-PROPOSAL.md) has the rule and
  `rahash` checks it against the collection: it never merges two images that
  run differently.

## Layout

```
crates/
  antic-core/      the machine: CPU, ANTIC, GTIA, POKEY, cartridges, HLE BIOS,
                   save states, the canonical hash image (no I/O)
  antic-libretro/  the libretro core (cdylib)
  antic-runner/    dev harnesses: `tomharte` and `klaus` (CPU tests), `shot`
                   (headless frames and audio, `--state` to redraw a save
                   state), `hlediff` (HLE against a real BIOS), `census`,
                   `statecheck`, `rahash`, `dis`, `dlist`, `antic`
docs/notes/        design notes and measurements
tools/rcheevos/    checks of the memory map and the hash through rcheevos
bios/, dumps/      a BIOS dump and cartridges for the harnesses (gitignored,
                   bring your own; the core itself needs neither)
```

## Building

```sh
# The libretro core
cargo build --release -p antic-libretro
# -> target/release/{libanticrust_libretro.so | anticrust_libretro.dll | libanticrust_libretro.dylib}

# The tests
cargo test --release

# What the core makes of a cartridge: size, CRC, mapping
cargo run --release -p antic-runner --bin antic -- path/to/game.a52
```

The core accepts `.a52`, `.bin` and `.rom`. It stamps the commit it was built
from into its feature string:

```sh
strings -a libanticrust_libretro.so | grep -o 'build=[A-Za-z0-9.-]*'
```

### Android

```sh
rustup target add aarch64-linux-android
cp .cargo/config.toml.example .cargo/config.toml
# edit .cargo/config.toml: point the linker at your NDK's clang wrapper

cargo build --release -p antic-libretro --target aarch64-linux-android
# -> target/aarch64-linux-android/release/libanticrust_libretro.so
```

The config pins `-Wl,-z,max-page-size=16384`, so the `.so` is 16 KB-aligned as
the Play Store requires for API 35+.

## References

Written from the Altirra Hardware Reference Manual. The 6502 core is shared
with [FamiRust](https://github.com/Prodigy75000/FamiRust), with decimal mode
added. The NTSC palette table is from
[a5200](https://github.com/libretro/a5200).

## License

GNU General Public License v3.0 or later. See [LICENSE](LICENSE).
