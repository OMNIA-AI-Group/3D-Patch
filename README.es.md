<p align="center"><img src="IMAGES/Logo3D/Logo-3D-Patch_34.png" width="160" alt="3D-Patch"></p>

<h1 align="center">3D-Patch</h1>

<p align="center">[English](README.md) · [Français](README.fr.md) · [Español](README.es.md) · [Italiano](README.it.md) · [Deutsch](README.de.md)</p>

<p align="center">Modelador 3D para impresión 3D — repara, modifica y prepara tus modelos en una sola aplicación libre.</p>

![3D-Patch](docs/capture.png)

## ¿Qué es 3D-Patch?

3D-Patch es un modelador 3D de escritorio al estilo de Microsoft 3D Builder, escrito en Rust sobre el núcleo geométrico industrial **OpenCASCADE**. Abre casi cualquier archivo 3D, detecta y repara los defectos que hacen fallar una impresión (agujeros, caras invertidas o rotas), permite modificar el objeto (cortar, combinar, vaciar, grabar, añadir texto, imágenes y texturas en relieve como grips) y lo envía a tu laminador listo para imprimir.

Funciona en Linux y Windows, completamente sin conexión, y su interfaz está disponible en francés, inglés, español, italiano y alemán.

## Funciones

- **Vistas**: texturizada, transparente, alámbrica y una vista de *errores y defectos* que resalta los problemas en color.
- **Reparación**: detección de agujeros y caras erróneas, limpieza interna, reparación automática en un clic y reparación «A.I.» que reconstruye una envolvente cerrada (estanca).
- **Edición**: booleanas (unión, resta, intersección), corte por un plano, vaciado con orificios de drenaje, simplificación, objetos alámbricos (celosía), grabado de relieve/textura, texto 3D, e imágenes vectorizadas en blanco y negro (umbral, automático, adaptativo o por colores, tolerancia ajustable, negativo, eliminación del marco) en relieve o grabadas como un texto.
- **Texturas en relieve**: 76 motivos integrados calculados por 3D-Patch (sin archivos de imagen), en pestañas — incluida una familia **Grip** de micropúas que se agarran a la piel y a los guantes (cuadriculado de arma, punteado, micropúas, puntas de diamante, moleteado, escofina, espinas, cinta antideslizante…) — y tus propias imágenes. Vista previa en directo, control de tamaño, botón *Negativo*, en relieve o grabado, envoltura triplanar / cubo / plana / cilindro / esfera, capas. Zonas lisas con el **Pincel**, por **Superficie** (un clic toma toda una cara, pequeños escalones de hasta 0,5 mm incluidos) o con la **Goma**; el clic derecho deshace la última exclusión. El motor adaptativo solo añade triángulos donde el relieve lo necesita, mantiene la pieza estanca y la base plana sobre la cama.
- **Colocación**: gizmo mover/girar/escalar, cotas exactas en mm/cm/pulgadas, centrado, regla, colocación en la cama con una simulación de gravedad realista, apoyo del objeto sobre cualquier cara plana (análisis de superficies con barra de progreso y lista de caras).
- **Piezas y grupos**: clic en una pieza de un objeto de varias piezas y clic derecho → *Desagrupar*; Mayús+clic en varios objetos y clic derecho → *Agrupar*; copiar/pegar (Ctrl+C / Ctrl+V), borrar con Supr o Retroceso.
- **Preparación para imprimir**: comprobaciones (estanqueidad, voladizos, paredes finas, tamaño), orientación recomendada, estimación del peso de material, envío directo a los laminadores instalados (PrusaSlicer, Cura, OrcaSlicer, Bambu Studio, SuperSlicer, Creality Print…).
- **Modo visor**: 3D-Patch puede ser la aplicación predeterminada para archivos 3D (doble clic para verlos).

## Formatos compatibles

Importación: STL, OBJ, PLY, 3MF, STEP, IGES, Rhino/MoI 3DM, SketchUp SKP, FBX, glTF/GLB, DAE, 3DS, X3D, FreeCAD FCStd, DXF, SolidCraft (.solidcraft), Blender (mediante un Blender instalado)… — Exportación: STL, 3MF (con colores), OBJ, PLY, Rhino/MoI 3DM, SketchUp SKP, 3DS, FBX, DAE, glTF/GLB, X3D, SolidCraft.

Diseños de SolidCraft: las mallas y las piezas STEP importadas se leen directamente, sin SolidCraft. Un diseño puramente paramétrico (bocetos, extrusiones, redondeos…) no contiene geometría: si SolidCraft está instalado, 3D-Patch se lo hace calcular y guarda el resultado para reabrir el mismo archivo al instante. Guardar en SolidCraft escribe cada objeto como una malla importada, que SolidCraft abre.

## Descarga

Las versiones listas para usar están en la página de [Releases](https://github.com/OMNIA-AI-Group/3D-Patch/releases):

| Archivo | Sistema | Instalación |
|---|---|---|
| `3D-Patch-x86_64.AppImage` | cualquier Linux x86-64 | `chmod +x` y doble clic |
| `3d-patch_<version>-1_amd64.deb` | Debian, Ubuntu, Mint, Zorin, Pop!_OS… | `sudo apt install ./3d-patch_<version>-1_amd64.deb` |
| `3d-patch-bin-<version>-1-x86_64.pkg.tar.zst` | Arch Linux, Manjaro, EndeavourOS… | `sudo pacman -U 3d-patch-bin-<version>-1-x86_64.pkg.tar.zst` (o `makepkg -si` con [`paquets/PKGBUILD`](paquets/PKGBUILD)) |
| `3d-patch-<version>-linux-x86_64.tar.gz` | cualquier Linux (manual) | extraer en `/`: `/opt/3d-patch` + lanzador |
| `3D-Patch-Setup.exe` | Windows 10/11 | instalador |
| `3D-Patch-Windows-portable.zip` | Windows 10/11 | descomprimir y ejecutar `3D-Patch\3d-patch.exe` |

## Compilar desde el código fuente (Linux)

Probado en Ubuntu 24.04 / Zorin OS 18 con Rust ≥ 1.80. Los pasos de Rhino 3DM y SketchUp son opcionales: sin ellos, 3D-Patch se compila sin esos dos formatos.

```bash
sudo apt install build-essential cmake ninja-build pkg-config libgtk-3-dev \
  libocct-foundation-dev libocct-modeling-data-dev libocct-modeling-algorithms-dev \
  libocct-data-exchange-dev libassimp-dev

git clone --recursive https://github.com/OMNIA-AI-Group/3D-Patch.git
cd 3D-Patch

# Opcional — soporte de Rhino 3DM (openNURBS)
git -C libs/opennurbs apply ../../patches/opennurbs.patch
cmake -S libs/opennurbs -B libs/opennurbs/build -DCMAKE_BUILD_TYPE=Release
cmake --build libs/opennurbs/build -j

# Opcional — soporte de SketchUp (OpenSKP)
git -C libs/skp-iamahsanmehmood apply ../../patches/openskp.patch
cmake -S libs/skp-iamahsanmehmood/packages/cpp -B libs/skp-iamahsanmehmood/packages/cpp/build \
  -G Ninja -DCMAKE_BUILD_TYPE=Release -DOPENSKP_BUILD_TESTS=OFF
cmake --build libs/skp-iamahsanmehmood/packages/cpp/build

cd 3D-Patch && cargo build --release
./target/release/3d-patch
```

La versión para Windows se compila de forma cruzada desde Linux con MinGW-w64 (ver `winbuild/mingw-w64.cmake` y el script NSIS `winbuild/3d-patch.nsi`).

## Estructura del repositorio

- `3D-Patch/` — la aplicación (Rust en `src/`, puentes C++ hacia OpenCASCADE, Assimp, openNURBS y OpenSKP en `cpp/`)
- `libs/` — openNURBS y OpenSKP (submódulos git), `patches/` — los pequeños cambios que se les aplican
- `IMAGES/` — logotipo y su fuente de Blender, `winbuild/` — empaquetado para Windows, `install-systeme.sh` — instalación local en `/opt`

## Licencia

3D-Patch es software libre publicado bajo la **Licencia Pública General de GNU v3.0** — ver [LICENSE](LICENSE). Puedes usarlo, estudiarlo, modificarlo y compartirlo; las versiones modificadas que distribuyas deben mantener la misma licencia.

Componentes de terceros: OpenCASCADE (LGPL-2.1), manifold-rust (Apache-2.0), openNURBS (licencia McNeel), OpenSKP (MIT), Assimp (BSD-3), egui · glow · winit · glam (MIT/Apache-2.0), rfd · image (MIT).

## Autor

**Alain Valette-Clary** — ΩiA-Group · [LinkedIn](https://www.linkedin.com/in/alain-valette-clary-524b28352/)

© 2026 Alain Valette-Clary / ΩiA-Group
