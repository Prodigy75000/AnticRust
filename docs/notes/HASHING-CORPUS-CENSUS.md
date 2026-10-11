# Hashing: what the corpus says, first pass

> **Superseded 2026-10-11.** The rule below was checked against every mapping
> and is now proposed, with its evidence, in `RA-PROPOSAL.md` ("Hashing").
> Kept as the first measurement it grew out of.

Measured 2026-10-11 over the staged corpus (215 files). Script: group by title,
md5 every file, compare variants byte for byte. Preliminary: it settles what
this corpus can settle and names what it cannot.

## Counts

| Folder | Files |
|---|---|
| Commercial (spelled "Commerical" in the archive) | 69 |
| Homebrew and ROM hacked | 117 |
| Prototype | 29 |

Extensions: 158 `.bin` (6 of them upper-case `.BIN`), 50 `.a52`, 7 `.rom`.
**No `.zip` files were found**, despite an earlier count of two.

Sizes: 4096 x2, 8192 x28, 16384 x85, 32768 x98, 40960 x1, 524288 x1. Every file
is a multiple of 4096.

## Finding 1: no file in this corpus carries a header

Every `.a52` is a raw power-of-two image, the same shape as the `.bin` files.
A 16-byte header would make sizes like 16400; none exist. So in this corpus the
extension carries no information and the hash can be over raw bytes.

**What this search could not see:** within the 69 commercial titles each title
appears exactly once, so there is no `.a52` / `.bin` pair of the SAME cartridge
to compare. "Headers do not differ between extensions" is untested here, not
disproven. Headered `.a52` files may exist elsewhere in the wild; if they do,
they are detectable by size (not a multiple of 4096) and the rule below should
strip them.

## Finding 2: the real hazard is layout, not headers

The same program appears at different sizes. Two shapes seen:

- **Plain mirroring.** `Blaster` 32K is the 16K image twice; `Q-bert-nobutton`
  exists at 8K, 16K and 32K, each the 8K image repeated.
- **Two-chip expansion.** For 16K images, the "(32k)" variant is
  `[lo8K, lo8K, hi8K, hi8K]`, not `16K + 16K`. Confirmed on four pairs
  (Crazy Otto, SuperPacMan, Pac-Man Bugs Fixed, Pirate Pete v1). This is the
  16K two-chip cartridge layout written out as a linear 32K load.

A hash over file bytes makes each of these a different game to RA.

## Candidate rule (superseded: see RA-PROPOSAL.md)

Hash the minimal image: while the image is larger than 4K and is exactly its
first half repeated, halve it; then, if it is 32K of form
`[A, A, B, B]` with 8K blocks, reduce to `A ++ B`. Headers, if ever seen,
are stripped first by size. This is a candidate only: it has to be checked
against the cartridge mapping the core actually implements, because a
genuine 32K cart that happens to have that shape would be wrongly reduced.

## Exact duplicates

212 distinct md5 over 215 files. Three duplicate pairs, all crossing a
naming line rather than being real variants, e.g. `Atari Diagnostics V2.3`
(commercial) is byte-identical to `Diagnostic Cart.bin` (homebrew).
