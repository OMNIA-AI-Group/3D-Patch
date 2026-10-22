<p align="center"><img src="IMAGES/Logo3D/Logo-3D-Patch_34.png" width="160" alt="3D-Patch"></p>

<h1 align="center">3D-Patch</h1>

<p align="center">[English](README.md) · [Français](README.fr.md) · [Español](README.es.md) · [Italiano](README.it.md) · [Deutsch](README.de.md)</p>

<p align="center">3D modeler for 3D printing — repair, edit and prepare your models in one free application.</p>

![3D-Patch](docs/capture.png)

## What is 3D-Patch?

3D-Patch is a desktop 3D modeler in the spirit of Microsoft 3D Builder, written in Rust on top of the industrial **OpenCASCADE** geometry kernel. It opens almost any 3D file, finds and repairs the defects that make a print fail (holes, inverted or broken faces), lets you modify the object (cut, combine, hollow, engrave, add text) and sends it to your slicer ready to print.

It runs on Linux and Windows, works fully offline, and its interface is available in French, English, Spanish, Italian and German.

## Features

- **Views**: textured, transparent, wireframe, and an *errors & defects* view that highlights problems in colour.
- **Repair**: hole and faulty-face detection, internal cleanup, one-click auto-repair, and an “A.I.” repair that rebuilds a closed (watertight) shell.
- **Editing**: booleans (union, subtraction, intersection), cutting along a plane, hollowing with drain holes, simplification, wireframe (lattice) objects, relief/texture engraving, 3D text.
- **Placement**: move/rotate/scale gizmo, exact dimensions in mm/cm/inch, centering, ruler, drop onto the bed with a realistic gravity simulation.
- **Parts & groups**: click one part of a multi-part object and right-click → *Ungroup*; Shift+click several objects and right-click → *Group*; copy/paste (Ctrl+C / Ctrl+V), delete with Del or Backspace.
- **Print preparation**: checks (watertight, overhangs, thin walls, size), suggested orientation, material weight estimate, direct sending to installed slicers (PrusaSlicer, Cura, OrcaSlicer, Bambu Studio, SuperSlicer, Creality Print…).
- **Viewer mode**: 3D-Patch can become the default application for 3D files (double-click to view).

## Supported formats

Import: STL, OBJ, PLY, 3MF, STEP, IGES, Rhino/MoI 3DM, SketchUp SKP, FBX, glTF/GLB, DAE, 3DS, X3D, FreeCAD FCStd, DXF, SolidCraft (.solidcraft), Blender (via an installed Blender)… — Export: STL, 3MF (with colours), OBJ, PLY, Rhino/MoI 3DM, SketchUp SKP, 3DS, FBX, DAE, glTF/GLB, X3D, SolidCraft.

SolidCraft designs: meshes and imported STEP parts are read directly, without SolidCraft. A purely parametric design (sketches, extrusions, fillets…) contains no geometry: if SolidCraft is installed, 3D-Patch has it computed, then keeps the result so the same file reopens instantly. Saving to SolidCraft writes each object as an imported mesh, which SolidCraft opens.

## Download

Ready-to-use builds (Linux AppImage and Windows installer) are on the [Releases](https://github.com/OMNIA-AI-Group/3D-Patch/releases) page.

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
