<p align="center"><img src="IMAGES/Logo3D/Logo-3D-Patch_34.png" width="160" alt="3D-Patch"></p>

<h1 align="center">3D-Patch</h1>

<p align="center">[English](README.md) · [Français](README.fr.md) · [Español](README.es.md) · [Italiano](README.it.md) · [Deutsch](README.de.md)</p>

<p align="center">Modellatore 3D per la stampa 3D — ripara, modifica e prepara i tuoi modelli in un'unica applicazione libera.</p>

![3D-Patch](docs/capture.png)

## Che cos'è 3D-Patch?

3D-Patch è un modellatore 3D per desktop nello spirito di Microsoft 3D Builder, scritto in Rust sul nucleo geometrico industriale **OpenCASCADE**. Apre quasi ogni file 3D, individua e ripara i difetti che fanno fallire una stampa (buchi, facce invertite o danneggiate), permette di modificare l'oggetto (tagliare, unire, svuotare, incidere, aggiungere testo) e lo invia al tuo slicer pronto per la stampa.

Funziona su Linux e Windows, completamente offline, e la sua interfaccia è disponibile in francese, inglese, spagnolo, italiano e tedesco.

## Funzionalità

- **Viste**: con texture, trasparente, wireframe e una vista *errori e difetti* che evidenzia i problemi a colori.
- **Riparazione**: rilevamento di buchi e facce errate, pulizia interna, riparazione automatica con un clic e riparazione «A.I.» che ricostruisce un involucro chiuso (stagno).
- **Modifica**: booleane (unione, sottrazione, intersezione), taglio lungo un piano, svuotamento con fori di drenaggio, semplificazione, oggetti a traliccio, incisione di rilievi/texture, testo 3D.
- **Posizionamento**: gizmo sposta/ruota/scala, quote esatte in mm/cm/pollici, centratura, righello, appoggio sul piatto con una simulazione realistica della gravità.
- **Preparazione alla stampa**: controlli (tenuta, sbalzi, pareti sottili, dimensioni), orientamento consigliato, stima del peso del materiale, invio diretto agli slicer installati (PrusaSlicer, Cura, OrcaSlicer, Bambu Studio, SuperSlicer, Creality Print…).
- **Modalità visualizzatore**: 3D-Patch può diventare l'applicazione predefinita per i file 3D (doppio clic per visualizzarli).

## Formati supportati

Importazione: STL, OBJ, PLY, 3MF, STEP, IGES, Rhino/MoI 3DM, SketchUp SKP, FBX, glTF/GLB, DAE, 3DS, X3D, FreeCAD FCStd, DXF, Blender (tramite un Blender installato)… — Esportazione: STL, 3MF (con colori), OBJ, PLY, Rhino/MoI 3DM, SketchUp SKP, 3DS, FBX, DAE, glTF/GLB, X3D.

## Download

Le versioni pronte all'uso (AppImage per Linux e installer per Windows) sono nella pagina [Releases](https://github.com/OMNIA-AI-Group/3D-Patch/releases).

## Compilare dai sorgenti (Linux)

Testato su Ubuntu 24.04 / Zorin OS 18 con Rust ≥ 1.80. I passaggi per Rhino 3DM e SketchUp sono facoltativi: senza di essi, 3D-Patch viene compilato senza questi due formati.

```bash
sudo apt install build-essential cmake ninja-build pkg-config libgtk-3-dev \
  libocct-foundation-dev libocct-modeling-data-dev libocct-modeling-algorithms-dev \
  libocct-data-exchange-dev libassimp-dev

git clone --recursive https://github.com/OMNIA-AI-Group/3D-Patch.git
cd 3D-Patch

# Facoltativo — supporto Rhino 3DM (openNURBS)
git -C libs/opennurbs apply ../../patches/opennurbs.patch
cmake -S libs/opennurbs -B libs/opennurbs/build -DCMAKE_BUILD_TYPE=Release
cmake --build libs/opennurbs/build -j

# Facoltativo — supporto SketchUp (OpenSKP)
git -C libs/skp-iamahsanmehmood apply ../../patches/openskp.patch
cmake -S libs/skp-iamahsanmehmood/packages/cpp -B libs/skp-iamahsanmehmood/packages/cpp/build \
  -G Ninja -DCMAKE_BUILD_TYPE=Release -DOPENSKP_BUILD_TESTS=OFF
cmake --build libs/skp-iamahsanmehmood/packages/cpp/build

cd 3D-Patch && cargo build --release
./target/release/3d-patch
```

La versione Windows è compilata in cross-compilazione da Linux con MinGW-w64 (vedi `winbuild/mingw-w64.cmake` e lo script NSIS `winbuild/3d-patch.nsi`).

## Struttura del repository

- `3D-Patch/` — l'applicazione (Rust in `src/`, ponti C++ verso OpenCASCADE, Assimp, openNURBS e OpenSKP in `cpp/`)
- `libs/` — openNURBS e OpenSKP (sottomoduli git), `patches/` — le piccole modifiche applicate a essi
- `IMAGES/` — logo e il suo sorgente Blender, `winbuild/` — pacchetto Windows, `install-systeme.sh` — installazione locale in `/opt`

## Licenza

3D-Patch è software libero distribuito con la **GNU General Public License v3.0** — vedi [LICENSE](LICENSE). Puoi usarlo, studiarlo, modificarlo e condividerlo; le versioni modificate che distribuisci devono restare sotto la stessa licenza.

Componenti di terze parti: OpenCASCADE (LGPL-2.1), manifold-rust (Apache-2.0), openNURBS (licenza McNeel), OpenSKP (MIT), Assimp (BSD-3), egui · glow · winit · glam (MIT/Apache-2.0), rfd · image (MIT).

## Autore

**Alain Valette-Clary** — ΩiA-Group · [LinkedIn](https://www.linkedin.com/in/alain-valette-clary-524b28352/)

© 2026 Alain Valette-Clary / ΩiA-Group
