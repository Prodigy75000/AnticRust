# The HLE BIOS: what it is, and how it is checked against the real one

Written 2026-10-11. The image is built by `crates/antic-core/src/hle.rs` and
`font.rs`. When `Machine::new` gets no dump, it runs this image. A real dump
is the oracle, and the harnesses below compare the two.

## Shape: one synthesised image

The census (`BIOS-CENSUS.md`) found that cartridges use the BIOS in two ways:
- **as code at fixed addresses:** vectors, jumps into $FCB2, $FCBD and $FD23,
  and copies of the pointer table;
- **as data:** the font and the tables.

So the HLE is a 2K image, not traps in the CPU loop. Our own 6502 routines
are assembled at runtime by a small assembler in `hle.rs`, at the entry
points the census measured, plus the font, the keypad table, the default
vector table and the hardware vectors. The entry-point table is at the top
of `hle.rs`.

**Where timing is behaviour, the layout follows the hardware.** My first IRQ
dispatcher was written for brevity, with a `JSR` to a shared
acknowledgement routine. Paths through it then took more cycles than the
real dispatcher's, and 26 titles diverged after their first keypad IRQ: a
game that reads RANDOM or VCOUNT after an IRQ sees the cycle count. The
dispatcher, the NMI dispatcher, the VBI and the default DLI now keep the
hardware's instruction order and branch layout. Tests pin each landmark
address, because a taken branch that crosses a page boundary costs a cycle.

**The font looks like the original.** The owner decided this on 2026-10-11.
At 8x8, "looks like" means the same pixels. `font.rs` holds the 128 glyphs as
text art, drawn once from the original by a script. Nothing reads a BIOS
file to build it.

## Boot

- **$BFFD = $FF:** entered at once, as on hardware (40 of 69).
- **Otherwise:** the HLE writes the post-boot state directly:
  - chip registers and zero page cleared;
  - the RAM vectors at $0200;
  - the measured shadows, CHBASE, SKCTL and NMIEN;
  - CPU registers A=$02 X=$FF Y=$1B SP=$FF P=$27.

  Then it enters the cart. There is no logo and no 4.25 s wait.

`entrystate` measured that state on the real BIOS and found it identical for
all 29 full-boot titles. `entrystate --hle` boots every title both ways and
compares zero page, $0200-$021F, the stateful chip registers and the CPU at
the cart's first instruction:

    69 of 69 titles enter with identical zero page, vectors, chip registers and CPU

That comparison was proven able to fail. Changing one post-boot colour value
by one makes it report all 29 full-boot titles.

**What the HLE boot does NOT reproduce:**
- **The logo's bytes in RAM.** That is about 850 bytes at $1000-$1136,
  $2000-$2059 and $3000-$3CFF.
- **The 255 frames of waiting.** A full-boot title therefore starts at a
  different beam position and a different point in POKEY's random sequence.
  Anything it randomises plays out differently from a run on the real BIOS.
  That is a different playthrough, not a fault.

Whether the boot should be emulated, skipped or configurable is **the
owner's decision and still open**. The measurement went to the orchestrator
on 2026-10-11.

## Run-time fidelity: `hlediff --swap`

Both runs boot on the real BIOS. At the cart's first instruction one of them
has the HLE image swapped in. Then both get the same scripted input:
- Start, fire, the top button and keypad 1, every 240 frames;
- the stick sweeping throughout.

Frames and RAM are compared after 1, 30, 120, 300, 900 and 1800 frames.

    67 of 69 titles identical at every checkpoint

Every frame is pixel-identical and all RAM is identical. The two that differ:

- **PAM SALT** reads the whole BIOS, as a diagnostics cart presumably does
  to checksum it. A different image should give a different result.
- **Gorf** jams on the REAL BIOS too, so the comparison has nothing to say.
  Gorf builds code in RAM between frames 101 and 120 after power-on. Pressing
  Start before that makes it call the code before it exists. With input from
  frame 300, as the census uses, it runs fine. Whether a real console does
  the same is unverified.

## Whole HLE, boot included: `hlediff`

    40 of 69 titles identical at every checkpoint

Every difference is a full-boot title, which is what the two boot gaps above
predict:
- the frames right after entry show the logo on the real BIOS and not on the
  HLE;
- later frames diverge wherever the game draws on randomness.

The swap run shows the run-time side is identical, and `entrystate --hle`
shows the handed-over state is identical. That leaves the boot gaps as the
only remaining cause.

## Reproduce

    cargo run --release -p antic-runner --bin entrystate -- --hle
    cargo run --release -p antic-runner --bin hlediff -- --swap
    cargo run --release -p antic-runner --bin hlediff
    cargo run --release -p antic-runner --bin boot -- <cart> --hle --script

Side-by-side PNGs of differing titles (real left, HLE right) go to
`out/hlediff/`.
