<p align="center"><img src="IMAGES/Logo3D/Logo-3D-Patch_34.png" width="160" alt="3D-Patch"></p>

<h1 align="center">3D-Patch</h1>

<p align="center">[English](README.md) · [Français](README.fr.md) · [Español](README.es.md) · [Italiano](README.it.md) · [Deutsch](README.de.md)</p>

<p align="center">3D modeler for 3D printing — repair, edit and prepare your models in one free application.</p>

![3D-Patch](docs/capture.png)

## What is 3D-Patch?

3D-Patch is a desktop 3D modeler in the spirit of Microsoft 3D Builder, written in Rust on top of the industrial **OpenCASCADE** geometry kernel. It opens almost any 3D file, finds and repairs the defects that make a print fail (holes, inverted or broken faces), lets you modify the object (cut, combine, hollow, engrave, add text, images and relief textures such as grips) and sends it to your slicer ready to print.

It runs on Linux and Windows, works fully offline, and its interface is available in French, English, Spanish, Italian and German.

## Features

- **Views**: textured, transparent, wireframe, and an *errors & defects* view that highlights problems in colour.
- **Repair**: hole and faulty-face detection, internal cleanup, one-click auto-repair, and an “A.I.” repair that rebuilds a closed (watertight) shell.
- **Editing**: booleans (union, subtraction, intersection), cutting along a plane, hollowing with drain holes, simplification, wireframe (lattice) objects, relief/texture engraving, 3D text, and images vectorised in black & white (threshold, automatic, adaptive or by colours, adjustable tolerance, negative, frame removal) embossed or engraved like text.
- **Relief textures**: 76 built-in patterns computed by 3D-Patch (no image files) in tabs — including a **Grip** family of sharp micro-points that bite into skin and gloves (gun checkering, stippling, micro-spikes, diamond points, knurling, rasp, thorns, grip tape…) — plus your own images. Live preview, size slider, *Negative* button, raised or engraved, triplanar / cube / flat / cylinder / sphere wrapping, layers. Keep areas smooth with the **Brush**, **Surface** (one click selects a whole face, small steps up to 0.5 mm included) or **Eraser**; right-click undoes the last exclusion. The adaptive engine only adds triangles where the relief needs them, keeps the part watertight and the bottom flat on the bed.
- **Placement**: move/rotate/scale gizmo, exact dimensions in mm/cm/inch, centering, ruler, drop onto the bed with a realistic gravity simulation, lay the object on any flat face (surface analysis with progress gauge and a list of faces).
- **Parts & groups**: click one part of a multi-part object and right-click → *Ungroup*; Shift+click several objects and right-click → *Group*; copy/paste (Ctrl+C / Ctrl+V), delete with Del or Backspace.
- **Print preparation**: checks (watertight, overhangs, thin walls, size), suggested orientation, material weight estimate, direct sending to installed slicers (PrusaSlicer, Cura, OrcaSlicer, Bambu Studio, SuperSlicer, Creality Print…).
- **Viewer mode**: 3D-Patch can become the default application for 3D files (double-click to view).

## Supported formats

Import: STL, OBJ, PLY, 3MF, STEP, IGES, Rhino/MoI 3DM, SketchUp SKP, FBX, glTF/GLB, DAE, 3DS, X3D, FreeCAD FCStd, DXF, SolidCraft (.solidcraft), Blender (via an installed Blender)… — Export: STL, 3MF (with colours), OBJ, PLY, Rhino/MoI 3DM, SketchUp SKP, 3DS, FBX, DAE, glTF/GLB, X3D, SolidCraft.

SolidCraft designs: meshes and imported STEP parts are read directly, without SolidCraft. A purely parametric design (sketches, extrusions, fillets…) contains no geometry: if SolidCraft is installed, 3D-Patch has it computed, then keeps the result so the same file reopens instantly. Saving to SolidCraft writes each object as an imported mesh, which SolidCraft opens.

## Download

Ready-to-use builds are on the [Releases](https://github.com/OMNIA-AI-Group/3D-Patch/releases) page:

| File | System | Install |
|---|---|---|
| `3D-Patch-x86_64.AppImage` | any Linux x86-64 | `chmod +x`, then double-click |
| `3d-patch_<version>-1_amd64.deb` | Debian, Ubuntu, Mint, Zorin, Pop!_OS… | `sudo apt install ./3d-patch_<version>-1_amd64.deb` |
| `3d-patch-bin-<version>-1-x86_64.pkg.tar.zst` | Arch Linux, Manjaro, EndeavourOS… | `sudo pacman -U 3d-patch-bin-<version>-1-x86_64.pkg.tar.zst` (or `makepkg -si` with [`paquets/PKGBUILD`](paquets/PKGBUILD)) |
| `3d-patch-<version>-linux-x86_64.tar.gz` | any Linux (manual) | extract into `/`: `/opt/3d-patch` + launcher |
| `3D-Patch-Setup.exe` | Windows 10/11 | installer |
| `3D-Patch-Windows-portable.zip` | Windows 10/11 | unzip, run `3D-Patch\3d-patch.exe` |

The Linux packages are built by [`paquets/construire-paquets.sh`](paquets/construire-paquets.sh).

## Building from source (Linux)

Tested on Ubuntu 24.04 / Zorin OS 18 with Rust ≥ 1.80. The Rhino 3DM and SketchUp steps are optional: without them, 3D-Patch builds without those two formats.

```bash
sudo apt install build-essential cmake ninja-build pkg-config libgtk-3-dev \
  libocct-foundation-dev libocct-modeling-data-dev libocct-modeling-algorithms-dev \
  libocct-data-exchange-dev libassimp-dev

git clone --recursive https://github.com/OMNIA-AI-Group/3D-Patch.git
cd 3D-Patch

# Optional — Rhino 3DM support (openNURBS)
git -C libs/opennurbs apply ../../patches/opennurbs.patch
cmake -S libs/opennurbs -B libs/opennurbs/build -DCMAKE_BUILD_TYPE=Release
cmake --build libs/opennurbs/build -j

# Optional — SketchUp support (OpenSKP)
git -C libs/skp-iamahsanmehmood apply ../../patches/openskp.patch
cmake -S libs/skp-iamahsanmehmood/packages/cpp -B libs/skp-iamahsanmehmood/packages/cpp/build \
  -G Ninja -DCMAKE_BUILD_TYPE=Release -DOPENSKP_BUILD_TESTS=OFF
cmake --build libs/skp-iamahsanmehmood/packages/cpp/build

cd 3D-Patch && cargo build --release
./target/release/3d-patch
```

The Windows build is cross-compiled from Linux with MinGW-w64 (see `winbuild/mingw-w64.cmake` and the NSIS script `winbuild/3d-patch.nsi`).

## Repository layout

- `3D-Patch/` — the application (Rust in `src/`, C++ bridges to OpenCASCADE, Assimp, openNURBS and OpenSKP in `cpp/`)
- `libs/` — openNURBS and OpenSKP (git submodules), `patches/` — the small changes applied to them
- `IMAGES/` — logo and its Blender source, `winbuild/` — Windows packaging, `install-systeme.sh` — local install into `/opt`

## Licence

3D-Patch is free software released under the **GNU General Public License v3.0** — see [LICENSE](LICENSE). You may use, study, modify and share it; modified versions you distribute must remain under the same licence.

Third-party components: OpenCASCADE (LGPL-2.1), manifold-rust (Apache-2.0), openNURBS (McNeel licence), OpenSKP (MIT), Assimp (BSD-3), egui · glow · winit · glam (MIT/Apache-2.0), rfd · image (MIT).

## Author

**Alain Valette-Clary** — ΩiA-Group · [LinkedIn](https://www.linkedin.com/in/alain-valette-clary-524b28352/)

© 2026 Alain Valette-Clary / ΩiA-Group
