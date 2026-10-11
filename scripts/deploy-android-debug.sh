#!/usr/bin/env bash
#
# Build the arm64 AnticRust libretro core, drop it into the TrophyHubAndroid
# app's jniLibs, and assemble the debug APK (optionally install it).
#
#   scripts/deploy-android-debug.sh            # build core -> jniLibs -> assembleDebug
#   scripts/deploy-android-debug.sh install    # ... then adb install -r the APK
#
# Lifted from ColecoRust's script of the same name (itself from MegaRust's);
# the crate, the .so name and the notes differ, and there is no Drive copy:
# the Drive holds exactly one debug APK, TrophyHub-debug.apk, and its notes
# file, which this script has no business overwriting. Only arm64-v8a ships.
#
# The Android linker comes from the umbrella's .cargo/config.toml (NDK clang,
# 16 KB page alignment), which covers every core under TrophyHub/.
#
# NOTE ON THE BIOS: there is none. The core always runs its own HLE image and
# never asks the frontend for a firmware file.
#
# NOTE ON SAVE STATES: fixed size, the same bytes on every platform.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID="$(cd "$REPO/../TrophyHubAndroid" && pwd)"
TRIPLE="aarch64-linux-android"
ABI="arm64-v8a"
SO="libanticrust_libretro.so"
# The commit, stamped into the .so (crates/antic-libretro/build.rs), so a
# device's core can be named: strings -a libanticrust_libretro.so | grep build=
ANTICRUST_BUILD_ID="$(git -C "$REPO" rev-parse --short=9 HEAD)"
if [ -n "$(git -C "$REPO" status --porcelain --untracked-files=no)" ]; then
  ANTICRUST_BUILD_ID="$ANTICRUST_BUILD_ID-dirty"
fi
export ANTICRUST_BUILD_ID
echo "[1/3] cargo build --release -p antic-libretro --target $TRIPLE  (build=$ANTICRUST_BUILD_ID)"
( cd "$REPO" && cargo build --release -p antic-libretro --target "$TRIPLE" )

SRC=""
for dir in "${CARGO_TARGET_DIR:-}" "$REPO/target" "$REPO/../target"; do
  [ -n "$dir" ] || continue
  if [ -f "$dir/$TRIPLE/release/$SO" ]; then
    SRC="$dir/$TRIPLE/release/$SO"
    break
  fi
done
[ -n "$SRC" ] || { echo "ERROR: built core not found in any target dir" >&2; exit 1; }
DST="$ANDROID/app/src/main/jniLibs/$ABI/$SO"
mkdir -p "$(dirname "$DST")"
echo "[2/3] cp $SRC -> $DST"
cp "$SRC" "$DST"
# Check the copy, not the build: what matters is what jniLibs now holds.
if ! grep -aq "build=$ANTICRUST_BUILD_ID" "$DST"; then
  echo "ERROR: $DST does not carry build=$ANTICRUST_BUILD_ID" >&2
  exit 1
fi
echo "      stamped build=$ANTICRUST_BUILD_ID"

echo "[3/3] ./gradlew :app:assembleDebug"
( cd "$ANDROID" && ./gradlew :app:assembleDebug )
APK="$ANDROID/app/build/outputs/apk/debug/app-debug.apk"
echo "APK: $APK"

if [ "${1:-}" = "install" ]; then
  echo "adb install -r (device must be authorized for USB debugging)"
  adb install -r "$APK"
fi
