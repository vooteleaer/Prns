#!/bin/sh
# Builds Prns for Haiku on Haiku: prnsd, and the Prns app (its status and
# controls, the Deskbar icon) as an app folder, haiku/build/Prns (the app,
# prnsd-start, config.default, bin/prnsd).
#
#     haiku/build.sh
#
# Needs: pkgman install rust_bin gcc binutils haiku_devel
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
OUT=$HERE/build/Prns
cd "$ROOT"
# RUST_MIN_STACK and --ignore-rust-version: see HAIKU.md.
RUST_MIN_STACK=16777216 cargo build --release --locked --ignore-rust-version \
	--manifest-path prnsd/Cargo.toml --no-default-features \
	--features tokio-cloud-host,observability
mkdir -p "$OUT/bin"
# A new file, not the old one overwritten: a running prnsd keeps its own.
rm -f "${OUT:?}/bin/prnsd"
cp prnsd/target/release/prnsd "$OUT/bin/"
cp "$HERE/prnsd-start" "$HERE/config.default" "$OUT/"
chmod +x "$OUT/prnsd-start"
# -rdynamic: Deskbar finds the replicant's Instantiate() among the app's
# exported symbols.
rm -f "${OUT:?}/Prns"
g++ -O2 -std=c++17 -Wall -o "$OUT/Prns" "$HERE/app/PrnsApp.cpp" \
	-I /boot/system/develop/headers/private/shared -lbe -lshared -rdynamic
rc -o "$HERE/build/prns.rsrc" "$HERE/app/prns.rdef"
xres -o "$OUT/Prns" "$HERE/build/prns.rsrc"
mimeset -f "$OUT/Prns"
echo "== $OUT"
ls -la "$OUT" "$OUT/bin"
