#!/bin/sh
# Builds img355ctl_<version>_amd64.deb from already compiled release binaries.
# usage: contrib/build-deb.sh <version> <dir with img355ctl and img355ctl-gui> [output dir]
set -eu

VERSION=$1
BIN=$2
OUT=${3:-.}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
umask 022
PKG=$(mktemp -d)
trap 'rm -rf "$PKG"' EXIT
chmod 755 "$PKG"  # mktemp makes it 0700; it becomes / on install

install -Dm 755 "$BIN/img355ctl" "$PKG/usr/bin/img355ctl"
install -Dm 755 "$BIN/img355ctl-gui" "$PKG/usr/bin/img355ctl-gui"
install -Dm 644 "$ROOT/contrib/70-img355-mouse.rules" "$PKG/usr/lib/udev/rules.d/70-img355-mouse.rules"
install -Dm 644 "$ROOT/contrib/img355ctl-gui.desktop" "$PKG/usr/share/applications/img355ctl-gui.desktop"
install -d "$PKG/usr/share/man/man1"
gzip -9n < "$ROOT/man/img355ctl.1" > "$PKG/usr/share/man/man1/img355ctl.1.gz"
install -Dm 644 "$ROOT/LICENSE" "$PKG/usr/share/doc/img355ctl/copyright"
install -Dm 644 "$ROOT/README.md" "$PKG/usr/share/doc/img355ctl/README.md"
install -Dm 644 "$ROOT/README.en.md" "$PKG/usr/share/doc/img355ctl/README.en.md"
if [ -f "$ROOT/THIRD-PARTY-LICENSES" ]; then
    gzip -9n < "$ROOT/THIRD-PARTY-LICENSES" > "$PKG/usr/share/doc/img355ctl/THIRD-PARTY-LICENSES.gz"
fi

install -d "$PKG/DEBIAN"
SIZE=$(du -sk --exclude=DEBIAN "$PKG" | cut -f1)
cat > "$PKG/DEBIAN/control" <<EOF
Package: img355ctl
Version: $VERSION
Architecture: amd64
Maintainer: Emre Yavuzalp <emreyavuzalp2@gmail.com>
Installed-Size: $SIZE
Depends: libc6 (>= 2.29), libgcc-s1
Recommends: libgl1, libegl1, libxkbcommon0, libwayland-client0 | libx11-6
Section: utils
Priority: optional
Homepage: https://github.com/EmreYavuzalp/Inca-IMG355GX-Linux
Description: Inca IMG-355GX gaming mouse configuration tool
 Configure the Inca IMG-355GX gaming mouse on Linux without the vendor's
 Windows software: DPI levels, LED colours and effects, button bindings,
 media keys and rapid-fire interval.
 .
 Includes the img355ctl command line tool, the img355ctl-gui graphical
 app and a udev rule that lets the logged-in user access the mouse.
EOF

cat > "$PKG/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = configure ] && command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules || true
    udevadm trigger --subsystem-match=hidraw --action=add || true
fi
EOF
cat > "$PKG/DEBIAN/postrm" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = remove ] && command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules || true
fi
EOF
chmod 755 "$PKG/DEBIAN/postinst" "$PKG/DEBIAN/postrm"

mkdir -p "$OUT"
dpkg-deb --root-owner-group -Zxz --build "$PKG" "$OUT/img355ctl_${VERSION}_amd64.deb"
