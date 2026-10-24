<p align="center"><img src="IMAGES/Logo3D/Logo-3D-Patch_34.png" width="160" alt="3D-Patch"></p>

<h1 align="center">3D-Patch</h1>

<p align="center">[English](README.md) · [Français](README.fr.md) · [Español](README.es.md) · [Italiano](README.it.md) · [Deutsch](README.de.md)</p>

<p align="center">Modeleur 3D pour l'impression 3D — réparez, modifiez et préparez vos modèles dans une seule application libre.</p>

![3D-Patch](docs/capture.png)

## Qu'est-ce que 3D-Patch ?

3D-Patch est un modeleur 3D de bureau dans l'esprit de Microsoft 3D Builder, écrit en Rust autour du noyau géométrique industriel **OpenCASCADE**. Il ouvre presque tous les fichiers 3D, détecte et répare les défauts qui font échouer une impression (trous, faces inversées ou cassées), permet de modifier l'objet (couper, assembler, évider, graver, ajouter du texte, des images et des textures en relief comme des grips) et l'envoie à votre trancheur prêt à imprimer.

Il fonctionne sous Linux et Windows, entièrement hors ligne, et son interface est disponible en français, anglais, espagnol, italien et allemand.

## Fonctionnalités

- **Vues** : texturée, transparente, filaire, et une vue *erreurs & défauts* qui montre les problèmes en couleur.
- **Réparation** : détection des trous et des faces en erreur, nettoyage interne, réparation automatique en un clic, et réparation « A.I » qui reconstruit une enveloppe fermée (étanche).
- **Modification** : booléens (union, soustraction, intersection), découpe selon un plan, évidement avec trous d'évacuation, simplification, objets filaires (treillis), gravure de relief/texture, texte 3D, et images vectorisées en noir & blanc (seuil, automatique, adaptatif ou par couleurs, tolérance réglable, négatif, suppression du cadre) incrustées en relief ou gravées comme un texte.
- **Textures en relief** : 76 motifs intégrés, calculés par 3D-Patch (aucun fichier image), rangés en onglets — dont une famille **Grip** de micro-pointes qui accrochent la peau et les gants (quadrillage d'arme, pointillage, micro-pointes, pointes de diamant, moletage, râpe, épines, ruban antidérapant…) — et vos propres images. Aperçu en direct, curseur de taille, bouton *Négatif*, relief ou gravé, enveloppe triplanaire / cube / à plat / cylindre / sphère, calques. Zones laissées lisses au **Pinceau**, par **Surface** (un clic prend toute une face, petites marches jusqu'à 0,5 mm comprises) ou à la **Gomme** ; le clic droit annule la dernière exclusion. Le moteur adaptatif n'ajoute des triangles que là où le relief l'exige, garde la pièce étanche et le dessous à plat sur le plateau.
- **Placement** : gizmo déplacer/tourner/redimensionner, cotes exactes en mm/cm/pouces, centrage, règle, pose sur le plateau avec une simulation de gravité réaliste, pose de l'objet sur n'importe quelle face plane (analyse des surfaces avec jauge d'avancement et liste des faces).
- **Pièces et groupes** : clic sur une pièce d'un objet à plusieurs pièces puis clic droit → *Dégrouper* ; Maj+clic sur plusieurs objets puis clic droit → *Grouper* ; copier/coller (Ctrl+C / Ctrl+V), suppression par Suppr ou Retour arrière.
- **Préparation à l'impression** : vérifications (étanchéité, surplombs, parois fines, dimensions), orientation conseillée, estimation du poids de matière, envoi direct aux trancheurs installés (PrusaSlicer, Cura, OrcaSlicer, Bambu Studio, SuperSlicer, Creality Print…).
- **Mode visionneuse** : 3D-Patch peut devenir l'application par défaut des fichiers 3D (double-clic pour les afficher).

## Formats pris en charge

Import : STL, OBJ, PLY, 3MF, STEP, IGES, Rhino/MoI 3DM, SketchUp SKP, FBX, glTF/GLB, DAE, 3DS, X3D, FreeCAD FCStd, DXF, SolidCraft (.solidcraft), Blender (via un Blender installé)… — Export : STL, 3MF (avec couleurs), OBJ, PLY, Rhino/MoI 3DM, SketchUp SKP, 3DS, FBX, DAE, glTF/GLB, X3D, SolidCraft.

Conceptions SolidCraft : les maillages et les pièces STEP importées sont lus directement, sans SolidCraft. Une conception purement paramétrique (esquisses, extrusions, congés…) ne contient pas de géométrie : si SolidCraft est installé, 3D-Patch la lui fait calculer, puis garde le résultat pour rouvrir le même fichier instantanément. L'enregistrement en SolidCraft écrit chaque objet comme un maillage importé, que SolidCraft ouvre.

## Téléchargement

Les versions prêtes à l'emploi sont sur la page [Releases](https://github.com/OMNIA-AI-Group/3D-Patch/releases) :

| Fichier | Système | Installation |
|---|---|---|
| `3D-Patch-x86_64.AppImage` | tout Linux x86-64 | `chmod +x`, puis double-clic |
| `3d-patch_<version>-1_amd64.deb` | Debian, Ubuntu, Mint, Zorin, Pop!_OS… | `sudo apt install ./3d-patch_<version>-1_amd64.deb` |
| `3d-patch-bin-<version>-1-x86_64.pkg.tar.zst` | Arch Linux, Manjaro, EndeavourOS… | `sudo pacman -U 3d-patch-bin-<version>-1-x86_64.pkg.tar.zst` (ou `makepkg -si` avec [`paquets/PKGBUILD`](paquets/PKGBUILD)) |
| `3d-patch-<version>-linux-x86_64.tar.gz` | tout Linux (manuel) | à extraire dans `/` : `/opt/3d-patch` + lanceur |
| `3D-Patch-Setup.exe` | Windows 10/11 | installeur |
| `3D-Patch-Windows-portable.zip` | Windows 10/11 | décompresser, lancer `3D-Patch\3d-patch.exe` |

Les paquets Linux sont fabriqués par [`paquets/construire-paquets.sh`](paquets/construire-paquets.sh).

## Compiler depuis les sources (Linux)

Testé sous Ubuntu 24.04 / Zorin OS 18 avec Rust ≥ 1.80. Les étapes Rhino 3DM et SketchUp sont facultatives : sans elles, 3D-Patch se compile sans ces deux formats.

```bash
sudo apt install build-essential cmake ninja-build pkg-config libgtk-3-dev \
  libocct-foundation-dev libocct-modeling-data-dev libocct-modeling-algorithms-dev \
  libocct-data-exchange-dev libassimp-dev

git clone --recursive https://github.com/OMNIA-AI-Group/3D-Patch.git
cd 3D-Patch

# Facultatif — prise en charge Rhino 3DM (openNURBS)
git -C libs/opennurbs apply ../../patches/opennurbs.patch
cmake -S libs/opennurbs -B libs/opennurbs/build -DCMAKE_BUILD_TYPE=Release
cmake --build libs/opennurbs/build -j

# Facultatif — prise en charge SketchUp (OpenSKP)
git -C libs/skp-iamahsanmehmood apply ../../patches/openskp.patch
cmake -S libs/skp-iamahsanmehmood/packages/cpp -B libs/skp-iamahsanmehmood/packages/cpp/build \
  -G Ninja -DCMAKE_BUILD_TYPE=Release -DOPENSKP_BUILD_TESTS=OFF
cmake --build libs/skp-iamahsanmehmood/packages/cpp/build

cd 3D-Patch && cargo build --release
./target/release/3d-patch
```

La version Windows est compilée depuis Linux avec MinGW-w64 (voir `winbuild/mingw-w64.cmake` et le script NSIS `winbuild/3d-patch.nsi`).

## Organisation du dépôt

- `3D-Patch/` — l'application (Rust dans `src/`, passerelles C++ vers OpenCASCADE, Assimp, openNURBS et OpenSKP dans `cpp/`)
- `libs/` — openNURBS et OpenSKP (sous-modules git), `patches/` — les petites modifications qui leur sont appliquées
- `IMAGES/` — logo et sa source Blender, `winbuild/` — empaquetage Windows, `install-systeme.sh` — installation locale dans `/opt`

## Licence

3D-Patch est un logiciel libre publié sous **licence publique générale GNU v3.0** — voir [LICENSE](LICENSE). Vous pouvez l'utiliser, l'étudier, le modifier et le partager ; les versions modifiées que vous distribuez doivent rester sous la même licence.

Composants tiers : OpenCASCADE (LGPL-2.1), manifold-rust (Apache-2.0), openNURBS (licence McNeel), OpenSKP (MIT), Assimp (BSD-3), egui · glow · winit · glam (MIT/Apache-2.0), rfd · image (MIT).

## Auteur

**Alain Valette-Clary** — ΩiA-Group · [LinkedIn](https://www.linkedin.com/in/alain-valette-clary-524b28352/)

© 2026 Alain Valette-Clary / ΩiA-Group
