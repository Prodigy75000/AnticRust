#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Prodigy75000
#
# Run AnticRust's libretro core through rcheevos' own memory code, twice:
#   stock  - the rcheevos our host ships, which has no Atari 5200 region
#            table, so it lays out SYSTEM_RAM from address 0;
#   table  - the same rcheevos with the region table proposed in
#            docs/notes/RA-PROPOSAL.md, so it reads SET_MEMORY_MAPS instead.
# Both must map RA address N to 6502 address N for all 16K.
#
#   tools/rcheevos/check.sh <cart>
#
# Needs gcc, a release build of the core, and a sibling TrophyHubLibretroHost
# checkout (rcheevos is taken from its origin/main, never its working tree).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
host="$root/../TrophyHubLibretroHost"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

git -C "$host" archive origin/main third_party/rcheevos | tar -x -C "$work"
R="$work/third_party/rcheevos"
# The patched copy gets the proposed region table (apply_5200.py, shared with
# hash-check.sh); the stock copy is left alone.
cp -r "$R" "$work/patched"
python "$here/apply_5200.py" "$work/patched" regions >/dev/null
P="$work/patched"

common="$R/src/rc_libretro.c $R/src/rc_compat.c $R/src/rc_util.c $(ls "$R"/src/rhash/*.c | tr '\n' ' ')"
inc="-I$R/include -I$R/src -I$R/src/rcheevos -I$R/test"
gcc -O1 -w $inc "$here/rc5200.c" "$R/src/rcheevos/consoleinfo.c" $common -o "$work/stock.exe"
gcc -O1 -w $inc "$here/rc5200.c" "$P/src/rcheevos/consoleinfo.c" $common -o "$work/table.exe"

dll="$root/target/release/anticrust_libretro.dll"
[ -f "$dll" ] || dll="$root/target/release/libanticrust_libretro.so"
echo "== stock rcheevos (no Atari 5200 table): SYSTEM_RAM path"
"$work/stock.exe" "$dll" "$1"
echo "== with the proposed region table: SET_MEMORY_MAPS path"
"$work/table.exe" "$dll" "$1"
