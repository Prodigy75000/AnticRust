#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Prodigy75000
#
# The proposed Atari 5200 hash, in rcheevos, against AnticRust's own.
# Applies tools/rcheevos/apply_5200.py to the host's origin/main rcheevos,
# hashes every corpus file through rc_hash_generate_from_buffer AND
# rc_hash_generate_from_file (which also exercises the "a52" routing), and
# compares both with `rahash --list` (MD5 of antic_core::hash::canonical).
#
#   tools/rcheevos/hash-check.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
host="$root/../TrophyHubLibretroHost"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

git -C "$host" archive origin/main third_party/rcheevos | tar -x -C "$work"
R="$work/third_party/rcheevos"
python "$here/apply_5200.py" "$R" all >/dev/null
srcs="$(ls "$R"/src/rhash/*.c "$R"/src/rcheevos/*.c | tr '\n' ' ') $R/src/rc_compat.c $R/src/rc_util.c $R/src/rc_libretro.c"
gcc -O1 -w -I"$R/include" -I"$R/src" -I"$R/src/rcheevos" -I"$R/test" "$here/hash5200.c" $srcs -o "$work/hash5200.exe"

( cd "$root" && cargo run --release -q -p antic-runner --bin rahash -- --list ) > "$work/rust.txt"
python - "$work" <<'PY'
import os, subprocess, sys
work = sys.argv[1]
rust = {}
for line in open(os.path.join(work, "rust.txt"), encoding="utf-8"):
    h, p = line.rstrip("\n").split(" ", 1)
    rust[os.path.normcase(os.path.abspath(p))] = h
paths = sorted(rust)
out = subprocess.run([os.path.join(work, "hash5200.exe")] + paths, capture_output=True, text=True).stdout.splitlines()
agree = 0
for line in out:
    hb, hf, p = line.split(" ", 2)
    r = rust[os.path.normcase(os.path.abspath(p))]
    if hb == r and hf == r:
        agree += 1
    else:
        print("MISMATCH", hb, hf, r, p)
print(f"{agree} of {len(out)} files: rcheevos buffer hash == rcheevos file hash == AnticRust canonical MD5")
sys.exit(0 if agree == len(out) and out else 1)
PY
