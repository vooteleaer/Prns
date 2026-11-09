#!/bin/sh
# Builds prns_<version>~retifs<N>_<arch>.deb: prnsd as every user's
# Reticulum shared instance (a systemd user service), for Debian 11+,
# Ubuntu 20.04+ (elementary OS 6+) and Raspberry Pi OS. Headless (no tray).
# A packaging of upstream Prns for retifs' testers until Prns publishes
# packages of its own.
#
#     retifs-packaging/deb/build-deb.sh [OUT] [amd64] [arm64]
#
# Built with cargo-zigbuild for glibc 2.31 (Debian 11, Ubuntu 20.04 and
# later) when it's there, else for this machine's glibc (arm64 then needs
# aarch64-linux-gnu-gcc).
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
OUT=${1:-$ROOT/target/deb}
shift || true
ARCHS=${*:-amd64 arm64}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target}
REV=${PRNS_DEB_REV:-1}
VERSION=$(tr -d '\r\n' < "$ROOT/VERSION")
DEBVER="$VERSION~retifs$REV"
GLIBC=${GLIBC:-2.31}
if command -v cargo-zigbuild >/dev/null; then
    BUILD="cargo zigbuild"; SUFFIX=".$GLIBC"
else
    BUILD="cargo build"; SUFFIX=""; GLIBC=$(ldd --version | sed -n '1s/.* //p')
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER:-aarch64-linux-gnu-gcc}
fi
mkdir -p "$OUT"

for ARCH in $ARCHS; do
    case $ARCH in
        amd64) TARGET=x86_64-unknown-linux-gnu ;;
        arm64) TARGET=aarch64-unknown-linux-gnu ;;
        *) echo "unknown arch $ARCH" >&2; exit 2 ;;
    esac
    echo "== $ARCH: building prnsd"
    $BUILD --release --locked --target $TARGET$SUFFIX --manifest-path "$ROOT/prnsd/Cargo.toml" \
        --no-default-features --features "tokio-host,observability"
    BIN=$CARGO_TARGET_DIR/$TARGET/release

    STAGE=$(mktemp -d)
    trap 'rm -rf "$STAGE"' EXIT
    chmod 0755 "$STAGE"
    install -d "$STAGE/DEBIAN" "$STAGE/usr/bin" "$STAGE/usr/lib/prns" "$STAGE/usr/lib/systemd/user" "$STAGE/usr/share/doc/prns"
    install -m 0755 "$BIN/prnsd" "$STAGE/usr/bin/prnsd"
    install -m 0755 "$HERE/no-shared-instance" "$STAGE/usr/lib/prns/"
    install -m 0644 "$HERE/prnsd.service" "$STAGE/usr/lib/systemd/user/"
    DOC=$STAGE/usr/share/doc/prns
    install -m 0644 "$ROOT/LICENSE-MIT" "$DOC/LICENSE-MIT"
    install -m 0644 "$ROOT/LICENSE-APACHE" "$DOC/LICENSE-APACHE"
    install -m 0644 "$ROOT/THIRD_PARTY_NOTICES.md" "$DOC/"
    install -m 0644 "$HERE/README.Debian" "$DOC/"
    { echo "prnsd: Copyright the Prns authors, MIT OR Apache-2.0 (see LICENSE-MIT, LICENSE-APACHE)."; } > "$DOC/copyright"

    SIZE=$(du -sk "$STAGE/usr" | cut -f1)
    cat > "$STAGE/DEBIAN/control" <<EOF
Package: prns
Version: $DEBVER
Architecture: $ARCH
Maintainer: retifs <https://github.com/vooteleaer/Prns>
Installed-Size: $SIZE
Depends: libc6 (>= $GLIBC)
Section: net
Priority: optional
Homepage: https://github.com/KenAKAFrosty/Prns
Description: Reticulum shared instance (prnsd), as a user service
 prnsd from Prns, a Rust implementation of Reticulum (RNS), started in every
 user's session as the machine's shared instance, unless one already runs
 (rnsd, or another prnsd). Apps (Sideband, NomadNet, retifs) attach to it.
 Its configuration is ~/.reticulum; the first start writes a default one.
 .
 Packaged for retifs' testers until Prns publishes packages of its own.
EOF
    install -m 0755 "$HERE/postinst" "$HERE/prerm" "$STAGE/DEBIAN/"
    DEB="$OUT/prns_${DEBVER}_${ARCH}.deb"
    dpkg-deb --root-owner-group -Zxz --build "$STAGE" "$DEB" >/dev/null
    rm -rf "$STAGE"
    trap - EXIT
    echo "== $DEB ($(du -h "$DEB" | cut -f1))"
done
