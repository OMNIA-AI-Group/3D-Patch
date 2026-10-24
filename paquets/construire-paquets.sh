#!/bin/bash
# Paquets Linux de 3D-Patch à partir du binaire compilé (cargo build --release) :
#   - archive portable  3d-patch-<v>-linux-x86_64.tar.gz  (arborescence /opt + /usr, source du PKGBUILD)
#   - Debian / Ubuntu   3d-patch_<v>-1_amd64.deb
#   - Arch Linux        3d-patch-bin-<v>-1-x86_64.pkg.tar.zst  (makepkg dans un conteneur Arch, Docker requis)
# Usage : ./paquets/construire-paquets.sh [dossier de sortie]
set -e
P="$(dirname "$(readlink -f "$0")")/.."
V=$(grep -m1 '^version' "$P/3D-Patch/Cargo.toml" | cut -d'"' -f2)
SORTIE="${1:-$P/LINUX PAQUETS}"
BIN="$P/3D-Patch/target/release/3d-patch"
[ -x "$BIN" ] || { echo "binaire absent : $BIN"; exit 1; }
mkdir -p "$SORTIE"
TRAV="$(mktemp -d)"
R="$TRAV/racine"
# --- arborescence installée ---
mkdir -p "$R/opt/3d-patch/bin" "$R/opt/3d-patch/lib" "$R/usr/bin" "$R/usr/share/applications" \
         "$R/usr/share/icons/hicolor/256x256/apps" "$R/usr/share/icons/hicolor/128x128/apps" "$R/usr/share/doc/3d-patch"
install -m755 "$BIN" "$R/opt/3d-patch/bin/3d-patch"
strip "$R/opt/3d-patch/bin/3d-patch"
cp -a "$P/install-libs"/. "$R/opt/3d-patch/lib/"
patchelf --force-rpath --set-rpath '$ORIGIN/../lib' "$R/opt/3d-patch/bin/3d-patch"
for f in "$R"/opt/3d-patch/lib/*.so*; do [ -L "$f" ] || patchelf --force-rpath --set-rpath '$ORIGIN' "$f"; done
chmod 644 "$R"/opt/3d-patch/lib/*
ln -s /opt/3d-patch/bin/3d-patch "$R/usr/bin/3d-patch"
install -m644 "$P/paquets/3d-patch.desktop" "$R/usr/share/applications/3d-patch.desktop"
python3 -I -c "from PIL import Image; i=Image.open('$P/IMAGES/Icone.png').convert('RGBA'); i.resize((256,256)).save('$R/usr/share/icons/hicolor/256x256/apps/3d-patch.png'); i.resize((128,128)).save('$R/usr/share/icons/hicolor/128x128/apps/3d-patch.png')"
install -m644 "$P/LICENSE" "$R/usr/share/doc/3d-patch/LICENSE"
# --- archive portable ---
tar -C "$R" --owner=0 --group=0 -czf "$SORTIE/3d-patch-$V-linux-x86_64.tar.gz" opt usr
# --- Debian / Ubuntu ---
D="$TRAV/deb"; cp -a "$R" "$D"; mkdir -p "$D/DEBIAN"
cat > "$D/DEBIAN/control" <<C
Package: 3d-patch
Version: $V-1
Section: graphics
Priority: optional
Architecture: amd64
Maintainer: Alain Valette-Clary <7956043+OMNIA-AI-Group@users.noreply.github.com>
Installed-Size: $(du -sk "$R" | cut -f1)
Depends: libc6, libgcc-s1, libstdc++6, zlib1g, libgl1, libegl1, libx11-6, libxkbcommon0, libxcursor1, libxi6, libxrandr2, libwayland-client0
Recommends: xdg-desktop-portal
Homepage: https://github.com/OMNIA-AI-Group/3D-Patch
Description: 3D modeler for 3D printing (repair, edit, textures, prepare)
 3D-Patch opens almost any 3D file, repairs printing defects, edits the
 object (booleans, cutting, hollowing, text, images, relief textures with
 grip patterns) and sends it to your slicer. Rust + OpenCASCADE.
C
fakeroot dpkg-deb --build --root-owner-group "$D" "$SORTIE/3d-patch_${V}-1_amd64.deb" >/dev/null
# --- Arch Linux (makepkg officiel, dans un conteneur Arch) ---
if command -v docker >/dev/null; then
    A="$TRAV/arch"; mkdir -p "$A"
    cp "$SORTIE/3d-patch-$V-linux-x86_64.tar.gz" "$A/"
    sed "s/^pkgver=.*/pkgver=$V/" "$P/paquets/PKGBUILD" > "$A/PKGBUILD"
    chmod -R a+rwX "$A"
    # réseau de l'hôte (le réseau par défaut des conteneurs peut être filtré) et plusieurs miroirs
    printf 'Server = %s/$repo/os/$arch\n' https://mirrors.kernel.org/archlinux https://archlinux.mirrors.ovh.net/archlinux \
        https://mirror.cyberbits.eu/archlinux https://mirrors.ircam.fr/pub/archlinux > "$A/mirrorlist"
    docker run --rm --network host -v "$A:/w" archlinux:latest bash -c '
        cp /w/mirrorlist /etc/pacman.d/mirrorlist &&
        pacman -Sy --noconfirm --needed base-devel >/dev/null 2>&1 &&
        useradd -m b && chown -R b /w &&
        su b -c "cd /w && makepkg -f --nodeps --skipchecksums" >/dev/null' 
    cp "$A"/3d-patch-bin-"$V"-1-x86_64.pkg.tar.zst "$SORTIE/"
fi
rm -rf "$TRAV"
ls -la "$SORTIE"
