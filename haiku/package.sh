#!/bin/sh
# Makes the Prns package for Haiku (an hpkg) from what build.sh made: prnsd
# on the PATH (every command, as on other systems: prnsd status, path, cp,
# interfaces, …), the Prns app with its Deskbar icon, prnsd and the icon as
# launch_daemon jobs at login, and an entry in the Deskbar's Applications
# menu.
#
#     haiku/build.sh && haiku/package.sh
#     pkgman install haiku/build/prns-<version>-x86_64.hpkg
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
APP=$HERE/build/Prns
STAGE=$HERE/build/package
VERSION=$(tr -d ' \r\n' < "$ROOT/VERSION" | tr '-' '_')-3
ARCH=x86_64
# Every package in a pkgman repository has its vendor: set it to the
# repository's when publishing in one (VENDOR=retifs haiku/package.sh).
VENDOR=${VENDOR:-Prns}

for f in Prns prnsd-start config.default bin/prnsd; do
	[ -f "$APP/$f" ] || { echo "missing $APP/$f: build it first" >&2; exit 1; }
done

rm -rf "${STAGE:?}"
mkdir -p "$STAGE/apps/Prns" "$STAGE/bin" "$STAGE/data/user_launch" \
	"$STAGE/data/deskbar/menu/Applications"
cp -a "$APP/Prns" "$APP/prnsd-start" "$APP/config.default" "$STAGE/apps/Prns/"
cp -a "$APP/bin/prnsd" "$STAGE/bin/prnsd"
ln -s ../../../../apps/Prns/Prns "$STAGE/data/deskbar/menu/Applications/Prns"
sed 's|APP|/boot/system/apps/Prns|g' "$HERE/launch/prns" \
	> "$STAGE/data/user_launch/prns"

cat > "$STAGE/.PackageInfo" <<EOF
name			prns
version			$VERSION
architecture	$ARCH
summary			"Prns: a Reticulum daemon (prnsd), with its status in the Deskbar"
description		"prnsd runs Reticulum for the programs on this computer (a shared \
instance on TCP ports 37428 and 37429), compatible with the Reticulum Network \
Stack. Its commands are on the PATH as on other systems: prnsd status, path, \
probe, id, cp, x, interfaces. The Prns app shows its interfaces and starts \
and stops it; its icon sits in the Deskbar. prnsd starts at login; a new \
installation begins with two public hubs (rns.beleth.net, rmap.world) and \
interface discovery, in ~/.reticulum/config."
packager		"Prns for Haiku"
vendor			"$VENDOR"
licenses {
	"MIT"
	"Apache v2"
}
copyrights {
	"Prns contributors"
}
provides {
	prns = $VERSION
	app:Prns = $VERSION
	cmd:prnsd = $VERSION
}
requires {
	haiku
	lib:libgcc_s
	lib:libstdc++
}
EOF

PKG=$HERE/build/prns-$VERSION-$ARCH.hpkg
rm -f "$PKG"
package create -C "$STAGE" "$PKG"
echo "== $PKG"
package list "$PKG" | tail -20
