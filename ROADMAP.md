# Roadmap — RevoScan Linux

Reimplementación para Linux de la app de escaneo 3D de Revopoint (POP 2 / POP 3),
en Rust, sobre el SDK oficial `lib3DCamera` vía FFI.

Leyenda: ✅ hecho · 🚧 en curso · ⬜ pendiente

---

## Fase 0 — Investigación y cimientos ✅

- [x] Investigar viabilidad y protocolo del escáner (UVC, VID `2207`, PIDs `110a/b/c`).
- [x] Descubrir el SDK oficial open-source (`Revopoint/3DViewer`) y la `lib3DCamera.so` de Linux.
- [x] Decidir arquitectura: **Rust + FFI** al SDK oficial (no reimplementar UVC).
- [x] Vendorizar el SDK (`vendor/3DCamera/`: `.so`, headers, reglas udev).
- [x] Esqueleto del proyecto Cargo + `build.rs` con enlazado y `RUNPATH` embebido.

## Fase 1 — Conectar y ver el stream ✅

- [x] Bindings FFI crudos (`src/sdk/ffi.rs`).
- [x] Wrapper seguro RAII `Session` (`src/sdk/mod.rs`): sistema → cámara → stream.
- [x] Hilo de captura con canales hacia la GUI (`src/capture.rs`).
- [x] GUI egui: visor de profundidad con colormap, estado, FPS, info de cámara, reconectar.
- [x] Script de instalación de reglas udev (`scripts/install-udev.sh`).
- [x] Compila, enlaza y arranca sin escáner (muestra estado); pendiente prueba con hardware real.

## Fase 2 — Stream RGB + vista emparejada 🚧

- [ ] Arrancar el stream RGB además del de profundidad.
- [ ] Usar `cameraGetPairedFrame` para obtener depth+RGB sincronizados.
- [ ] Decodificar el formato RGB (MJPG / H264 / RGB8) a imagen mostrable.
- [ ] GUI: vista lado a lado (profundidad | RGB), selector de stream.
- [ ] Manejar cámaras sin sensor RGB (MINI sin RGB) con elegancia.

## Fase 3 — Nube de puntos + exportar ⬜

- [ ] Leer intrínsecos/extrínsecos del SDK (`cameraGetStreamIntrinsics`, `...Extrinsics`).
- [ ] Generar nube de puntos desde el mapa de profundidad (deproyección).
- [ ] Visor 3D de la nube (cámara orbital) en la GUI.
- [ ] Colorear puntos con el frame RGB (mapeo depth→RGB con extrínsecos).
- [ ] Exportar a **PLY** (un frame).

## Fase 4 — Captura multi-frame y registro ⬜

- [ ] Buffer/grabación de secuencia de frames durante el escaneo.
- [ ] Registro/alineado entre frames (ICP).
- [ ] Fusión incremental (acumulación de nubes / TSDF).
- [ ] Controles de escaneo: iniciar/pausar/detener, contador de frames.

## Fase 5 — Malla y texturizado ⬜

- [ ] Reconstrucción de malla (p. ej. Poisson) desde la nube fusionada.
- [ ] Simplificación/limpieza de malla.
- [ ] Texturizado desde los frames RGB.
- [ ] Exportar **OBJ / STL**.

## Fase 6 — Controles de cámara y calidad ⬜

- [ ] Exposición, ganancia, rango de profundidad (`cameraSetProperty`).
- [ ] Modos HDR y de disparo (continuo / software / hardware).
- [ ] Auto-exposición, balance de blancos (RGB).
- [ ] Perfiles por modelo (POP 2 vs POP 3).

## Fase 7 — Distribución ⬜

- [ ] Empaquetado (AppImage / Flatpak) con la `.so` y reglas udev.
- [ ] Instalador que registre las reglas udev automáticamente.
- [ ] CI (build + lint) y releases.

---

## Notas para el repositorio git

- `lib3DCamera.so` pesa ~94 MB. GitHub avisa con archivos >50 MB y rechaza >100 MB.
  Opciones: **Git LFS** (recomendado), o no versionarla y dar un script que la
  descargue desde `Revopoint/3DViewer`. Decidir antes del primer push.
- `reference-3DViewer/` (clon de referencia) y `/target` están en `.gitignore`.
- Licencia: GPL-3.0 (coherente con el SDK redistribuido).
