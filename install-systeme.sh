#!/bin/bash
# Installe / met à jour 3D-Patch sur le système (comme un paquet) à partir du binaire de dev.
# Usage : ./install-systeme.sh   (cargo build --release fait au préalable)
set -e
P="$(dirname "$(readlink -f "$0")")"
BIN="$P/3D-Patch/target/release/3d-patch"
LIBS="$P/install-libs"   # bibliothèques embarquées (OCCT, assimp, tbb…)
[ -x "$BIN" ] || { echo "binaire absent : $BIN"; exit 1; }
sudo mkdir -p /opt/3d-patch/bin /opt/3d-patch/lib
sudo install -m755 "$BIN" /opt/3d-patch/bin/3d-patch
sudo strip /opt/3d-patch/bin/3d-patch
sudo cp -a "$LIBS"/. /opt/3d-patch/lib/
sudo patchelf --force-rpath --set-rpath '$ORIGIN/../lib' /opt/3d-patch/bin/3d-patch
for f in /opt/3d-patch/lib/*.so*; do [ -L "$f" ] || sudo patchelf --force-rpath --set-rpath '$ORIGIN' "$f"; done
sudo chown -R root:root /opt/3d-patch
sudo chmod 755 /opt/3d-patch /opt/3d-patch/bin /opt/3d-patch/lib
sudo chmod 644 /opt/3d-patch/lib/*
sudo ln -sf /opt/3d-patch/bin/3d-patch /usr/local/bin/3d-patch
sudo update-desktop-database /usr/share/applications
echo "3D-Patch installé dans /opt/3d-patch"
