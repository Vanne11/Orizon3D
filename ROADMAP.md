# Roadmap — Orizon3D

Aplicación de escaneo 3D para escáneres **Revopoint POP 2 / POP 3** en **Linux**,
escrita en **Rust**. Revopoint no publica software para Linux; este proyecto lo
reconstruye desde cero.

Leyenda: ✅ hecho · 🚧 en curso · ⬜ pendiente · ⭐ prioridad alta

---

## Arquitectura actual (leer esto primero)

**La captura se hace por V4L2 directo, NO por el SDK oficial.** El SDK
`lib3DCamera` exige una **licencia de pago por número de serie** que se provisiona
en el dispositivo (sin ella, `systemConnectCamera` falla con «authorize failed»).
Por eso se abandonó la vía FFI y se lee la cámara como dispositivo **UVC/V4L2**,
que no necesita licencia.

- El POP 2/3 expone: **profundidad `Y16` (16-bit, = Z16)** y **color `MJPG`**.
- Módulos:
  - `src/camera.rs` — descubrimiento de nodos V4L2, decodificación (Y16/MJPG),
    intrínsecos por FOV real.
  - `src/capture.rs` — hilo de captura V4L2 (crate `v4l`), canal de comandos
    (exposición/ganancia).
  - `src/pointcloud.rs` — deproyección a nube, recorte (ROI), visor 3D por
    software, export PLY.
  - `src/scan.rs` — ICP (frame-a-modelo, trimmed) + fusión por vóxeles + limpieza
    y aislado del objeto.
  - `src/mesh.rs` — malla (Surface Nets), relleno de huecos, decimación, visor y
    export OBJ/STL/PLY.
  - `src/app.rs` — GUI egui (visor + nube/malla en vivo, HUD, controles).
- `src/sdk/` (FFI al `.so`) se conserva como **referencia histórica**, fuera del
  árbol de módulos y sin enlazar (`build.rs` es no-op).
- **Calibración:** intrínsecos a partir del **FOV real** del POP 2 (HFOV≈30.7°,
  sensor 1280×800) + **ajuste fino** en la UI, persistido en `calibration.txt`.
  NO es la calibración de fábrica (vive tras el HID propietario).

---

## Lo hecho (Fases 0–5)

### Fase 0 — Investigación y cimientos ✅
- Protocolo del escáner (UVC, VID `2207`, PIDs `110a/b/c`), arquitectura, esqueleto Cargo.
- Descubrimiento de que el SDK exige licencia → **pivote a V4L2**.

### Fase 1 — Conectar y ver profundidad ✅ (vía V4L2)
- Descubrimiento de nodos V4L2, stream de profundidad `Y16`, colormap, FPS, estado, reconectar.

### Fase 2 — Color + vista emparejada ✅ (vía V4L2)
- Stream RGB `MJPG` decodificado (zune-jpeg), vista cámara (profundidad + color).
- ⬜ Selector de resolución/FPS · ⬜ H264.

### Fase 3 — Nube de puntos + export ✅
- Deproyección a nube, visor 3D orbital, color por punto, export **PLY (ASCII)**.
- ⬜ PLY binario.

### Fase 4 — Captura multi-frame y registro ✅
- **ICP frame-a-modelo**, **trimmed** (descarta el 30% peor) + **rechazo de
  frames mal alineados** (no fusiona basura), fusión por vóxeles, controles de
  escaneo + stats (seguimiento, frames, descartados).
- ⬜ Optimización global de poses / loop closure · ⬜ TSDF.

### Fase 5 — Malla y texturizado ✅ (base)
- Malla por **Surface Nets**, **relleno de huecos** (cierre morfológico),
  **decimación** (clustering), color por vértice, visor de malla, export
  **OBJ / STL / PLY**.
- ⬜ Textura UV real · ⬜ watertight/Poisson · ⬜ suavizado.

### Transversal (UI/UX) ✅ (base)
- Visor cámara + nube/malla simultáneos, HUD (distancia, cobertura, seguimiento),
  **caja de escaneo** (rango + recorte lateral), **detección de objeto**
  (denoise + aislar grupo mayor), **exposición/ganancia** de profundidad,
  **calibración** (FOV + ajuste fino), panel con scroll.

---

## Lo que falta y CÓMO mejorarlo

Ordenado por área; ⭐ = lo que más impacto tendría.

### A. Registro / calidad del escaneo  ⭐ (lo más importante)
El cuello de botella real. Con ICP geométrico puro, los objetos lisos/simétricos
"resbalan".
- ⭐ **Modo marcadores** — detectar marcadores (círculos reflectantes) en IR/RGB,
  emparejarlos entre frames y registrar por correspondencias (Kabsch). Es la vía
  robusta para objetos sin relieve; es lo que usa RevoScan para lo difícil.
- ⭐ **ICP punto-a-plano** — estimar normales del modelo (PCA sobre vecinos con el
  `VoxelIndex`) y minimizar distancia punto-a-plano (sistema lineal 6×6
  linealizado). Converge mucho mejor y reduce el resbalón tangencial.
- **Predicción de movimiento** — inicializar el ICP extrapolando la pose anterior
  (velocidad), no con la última pose estática.
- **Optimización global de poses (loop closure)** — grafo de poses; al cerrar el
  giro corrige la deriva acumulada. Complejo pero es el salto de "bonito" a "métrico".
- **Fusión TSDF** — en vez de promediar posición/color por vóxel, integrar una
  función de distancia con signo ponderada por normal/confianza → superficie más
  limpia y mejor base para la malla.
- **Quitar el plano de apoyo (mesa) automático** — RANSAC de plano para descartar
  la base sin depender solo de la caja.
- **Validación con hardware real** de cada mejora (medir deriva, RMSE, cierre).

### B. Calibración
- **Autocalibración con tablero de ajedrez** — estimar fx, fy, cx, cy **y
  distorsión** reales (detección de esquinas + solver). Lo más preciso sin la de fábrica.
- **Modelo de distorsión** — ahora se asume 0; añadir radial/tangencial.
- **Extrínsecos depth↔RGB reales** — ahora identidad; calibrar para que el color
  caiga bien sobre la geometría (mejor textura).
- **Extraer `camparam` de fábrica por HID** — ingeniería inversa del protocolo del
  dispositivo (difícil/incierto, pero sería la calibración exacta).
- **Verificar `depth_scale`** (mm/unidad) contra mediciones reales.

### C. Cámara / controles (Fase 6)
- Exposición/ganancia **RGB** (ahora solo profundidad) + auto-exposición.
- **Selector de resolución/FPS** (640×400 vs 1280×800; depth y color).
- **HDR / multi-exposición** para superficies oscuras o brillantes.
- Soporte **H264** (algunos modelos/streams).
- **Perfiles por modelo** (POP 2 vs POP 3) y por tipo de objeto.
- Reaplicar exposición/ajustes tras reconectar.

### D. Malla y textura (Fase 5+)
- ⭐ **Textura UV real** — atlas de textura proyectando los frames RGB por
  triángulo (selección de mejor vista, empaquetado UV). Hoy es color por vértice.
- **Malla watertight / Poisson** — reconstrucción con normales orientadas para
  una superficie más fiel que Surface Nets; opción "rellenar para impresión".
- **Decimación con métrica** (quadric edge collapse) en vez de clustering, para
  conservar detalle donde importa.
- **Suavizado** (Laplaciano/Taubin) y **rellenado de huecos por bordes** (detectar
  bucles de borde y triangular).
- **Normales por vértice** → sombreado suave en el visor y en el export.
- Formatos extra (**glTF/GLB** con textura).

### E. Nube de puntos / filtros
- **Outliers estadísticos** (k vecinos / desviación) además del filtro de densidad.
- **Edición en el visor** (lazo/borrar regiones) para limpiar a mano.
- **Export PLY binario** (más rápido y ligero que ASCII).
- Submuestreo (voxel) configurable antes de exportar.

### F. UI / UX (paridad con RevoScan)
- ⭐ **Dibujar la caja de escaneo** (wireframe) en el visor 3D para colocarla a ojo.
- **Visor 3D por GPU (wgpu)** en vez de rasterizado por software → fluido con
  nubes/mallas grandes, sombreado y normales reales.
- **Presets** (objeto / cara / cuerpo / oscuro) que fijen rango, exposición y modo.
- **Guardar/cargar proyecto** de escaneo (frames + poses + nube).
- Deshacer, línea de tiempo de frames, capturas.
- Indicadores visuales de calidad/cobertura sobre el modelo.

### G. Rendimiento
- ⭐ **Sacar el procesamiento del hilo de la GUI** — hoy ICP/fusión/limpieza
  corren en el `update()` de egui y pueden trabar la interfaz. Mover a un worker.
- **Paralelizar** (crate `rayon`) deproyección, ICP (búsqueda de vecinos) y mallado.
- **Reconstruir el modelo de ICP cada N frames** (no cada frame) para escalar.
- Limitar/streaming de la nube en el visor cuando crece mucho.

### H. Robustez y distribución (Fase 7)
- **Validación sistemática con hardware real** (transversal a todo).
- Empaquetado **AppImage / Flatpak** (sin SDK; solo el binario + reglas udev).
- udev: paquete de reglas + documentar acceso a `/dev/video*` (grupo `video` / uaccess).
- **CI** (build + `clippy` + `cargo test`) y releases automáticas.
- Reconexión y manejo de errores más robustos (hotplug, pérdida de stream).

### I. Alternativa para calidad completa hoy
- **VM de Windows + USB passthrough** (KVM/QEMU o VirtualBox) ejecutando el
  **RevoScan real** → calidad completa mientras la app nativa madura. (Wine no
  sirve: solo conecta por WiFi y el POP 2 es solo-USB.)

---

## Notas del repositorio
- La `lib3DCamera.so` (~94 MB) **ya no se enlaza** (captura por V4L2). Sigue
  descargable con `scripts/fetch-sdk.sh` solo como referencia; `src/sdk/` queda
  fuera del build.
- Dependencia nueva: **`v4l`** (ioctls V4L2, sin libv4l).
- Ignorados en git: `/target`, `/reference-3DViewer`, `/captures`,
  `vendor/3DCamera/lib/lib3DCamera.so`, `/calibration.txt`.
- Licencia del proyecto: GPL-3.0.
- Gotchas de hardware (uvcvideo, licencia, V4L2, etc.): ver
  `memory/revoscan-hardware-gotchas.md`.
