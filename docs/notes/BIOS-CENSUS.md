# BIOS census: what the 69 commercial titles use of the real BIOS

Measured 2026-10-11 with `cargo run --release -p antic-runner --bin census`
(`crates/antic-runner/src/census.rs`). Every commercial file runs on the real
4-port BIOS (md5 281f20ea..., the common one per Altirra's firmware database)
for 1800 frames, 30 seconds, with **no input**. A probe sees every bus cycle,
including ANTIC's DMA. The full per-title report is written to `out/census.txt`,
which is gitignored. It takes about 10 seconds for the whole set.

This document gives addresses and counts only. The BIOS bytes are not ours to
publish.

## What this measurement could not see

Read the numbers with these limits in mind:

- **No input, attract mode only.** Nothing presses a key, so keypad IRQs and
  game-play code paths are not exercised. The IRQ vector count below is zero
  for that reason. It is not a finding that games never use IRQs. A re-run
  with scripted input is owed.
- **No renderer yet.** "Ran for 30 seconds without jamming and without a BRK
  storm" is the bar every title cleared. It does not show that the emulation
  is correct.
- **RAM dependencies are candidates.** "The cart read a byte whose last
  writer was the BIOS" includes reads whose value never mattered, for example
  a clear loop that reads first. Re-running in HLE mode and diffing is what
  settles them.
- **Power-on RAM is zero here.** On hardware it is not reliably zero. Titles
  that skip the boot inherit whatever RAM holds.

## Boot

| | titles |
|---|---|
| $BFFD = $FF: the BIOS jumps straight to the cart | 40 |
| full boot: init, logo screen, then the cart | 29 |

The skip path runs `SEI`, `CLD` and sets the stack, then jumps through $BFFE.
That is 30 cycles. It sets up no hardware and no RAM vectors.

Every full-boot title takes the same time:

| phase | cycles | frames | share |
|---|---|---|---|
| setup: reset until the logo wait loop | 121,490 | 4.07 | **1.57%** |
| visible wait: logo until the first cart fetch | 7,613,171 | 254.89 | 98.43% |

The wait is the BIOS spinning until the VBI's frame counter at $02 reaches
$FF. During the wait the VBI keeps running, so the state the cart inherits
includes the counters and shadows as the wait leaves them.

## Cartridge mapping, measured

| mapping | titles |
|---|---|
| 4K/8K mirrored | 20 |
| 16K two-chip | 25 |
| 16K one-chip | 15 |
| 32K linear | 8 |
| 40K Bounty Bob banking | 1 |

Each 16K file ran under both mappings:
- **37 of 40**: the wrong mapping jammed the CPU or fell into a BRK storm.
- **Wizard of Wor and Zone Ranger**: both mappings ran, but one-chip executed
  about 20x more distinct code (711 vs 35, 1976 vs 72).
- **Star Wars - The Arcade Game**: it only executed code at addresses both
  mappings place identically, so this run cannot tell them apart.

The size does not decide the mapping. This is why the hashing rule
(`HASHING-CORPUS-CENSUS.md`) cannot be finalised without it.

## Control transfers into the BIOS

The NMI vector lives in BIOS ROM, so every title using NMIs (66 of 69) goes
through the BIOS dispatcher. The census followed control as it left the
BIOS. The number is how many titles took each exit:

| BIOS exit | meaning | titles |
|---|---|---|
| $FCAF | `JMP ($0206)`, the DLI vector | 57 |
| $FD2F | `JMP ($BFFE)` on the skip path | 40 |
| $FCFF | `JMP ($0204)`, the deferred VBI after the BIOS VBI ran | 40 |
| $FCAC | `JMP ($0202)`, the cart replaces the immediate VBI | 27 |
| $FCB7 | `RTI` back into the cart | 27 |

Direct calls (JMP or JSR from cart code into the BIOS, not interrupts):

| target | what is there | titles |
|---|---|---|
| $FCB2 | the shared tail that restores Y, X, A and returns from the interrupt | 14 |
| $FCBD | inside the BIOS VBI, after it saves registers | 1 |

Across all titles, the BIOS executed only **70 distinct instructions** after
entry:
- the NMI dispatcher and the VBI ($FCA2-$FCFF);
- the default DLI handler at $FEA1-$FEC5, which is pointed to by the BIOS's
  vector table.

## The BIOS read as DATA by cart code

26 titles read the BIOS region with ordinary loads:

- **Font, $F800-$FBFF: 21 titles.** They copy glyphs into their own
  character sets. Galaxian copies the whole font; River Raid, KABOOM!,
  Keystone Kapers, Megamania and others copy letters and digits.
- **The default RAM-vector table, $FE95-$FEA0: 4 titles** (K-razy Shoot-Out,
  Mountain King, Pitfall!, Congo Bongo). These are skip-boot titles, which get
  no vector setup, so they copy the BIOS's table into $0200 themselves.
- **The whole BIOS, $F800-$FFFF: 4 titles.** Two are diagnostics carts (Atari
  Diagnostics, PAM SALT) and presumably checksum it. **Two are games: The
  Dreadnaught Factor and Super Cobra.** Whether they act on what they read is
  the first thing the HLE diff must answer.
- **Bounty Bob Strikes Back: one byte, $FD10.** Altirra notes the 2-port BIOS
  revision moves code in this area, so this may be a version check.

Separately, **43 titles point ANTIC at the BIOS font** and display it
directly, through DMA.

## State the cart inherits

- **Runtime RAM the BIOS VBI maintains**, read by the cart:
  - $02 (frame counter): 20 titles;
  - $04: 17;
  - pot shadows $11-$18: up to 15;
  - $01: 5.
- **BIOS VBI writing hardware after entry: 42 titles.** The VBI copies
  shadows to DMACTL, DLIST and the nine colour registers every frame. Those
  titles set the shadows and let the BIOS write the chips.
- **Stack convention: 39 titles.** The cart's handler pulls registers that
  the BIOS VBI pushed.
- **Non-zero registers set at boot that the cart never rewrites:**
  - DLISTH = $20: 20 titles;
  - a colour register = $1D: 17;
  - another colour register = $0F: 9;
  - DMACTL = $22: 9;
  - CHBASE = $F8: 6;
  - SKCTL = $02: 3.

## What this says about the HLE

The intercept is **both**, as ColecoRust found for its machine, and for the
same kind of reason:

1. **The BIOS's code addresses are an interface.** Vectors point at them,
   14 titles jump to $FCB2, and 4 titles copy a table of pointers to them.
   The NMI dispatcher, the VBI with its shadow copies, the default DLI handler
   and that table must exist at the same addresses.
2. **The font is data that 43 titles display and 21 copy.** It needs a font
   in the same character order at $F800. That means a clean-room font drawn
   for this project, which will look different from the original. **Whether
   that is acceptable is the owner's call.**
3. Given 1 and 2, the natural HLE is a **synthesised 2K image**: our own
   6502 code at the documented entry points, our own font, and the same
   vector table. That is one image rather than traps in the CPU loop. The
   census, re-run against it, is the diff harness.
4. **Boot.** Skip-boot titles (40) need only a stack and flags. Full-boot
   titles (29) need the post-boot state:
   - hardware and zero page cleared;
   - RAM vectors at $0200;
   - CHBASE, SKCTL and the VBI running.

   That setup is 1.57% of the real boot's time. The other 98.43% is a logo
   on screen.
