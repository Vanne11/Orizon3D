# Orizon3D

Aplicación de escaneo 3D para escáneres **Revopoint POP 2 / POP 3** en **Linux**,
escrita en **Rust**. Revopoint no publica su software de escaneo para Linux, así
que Orizon3D lo reconstruye desde cero: captura, visor, nube de puntos, registro
multi-frame y mallado, todo nativo.

La captura usa **V4L2 directamente** (el escáner es un dispositivo UVC estándar:
profundidad `Y16` + color `MJPG`), sin depender del SDK propietario ni de
licencias. La GUI es **egui/eframe** y el visor 3D es un rasterizador por software
(sin dependencias gráficas extra).

## Estado

- [x] **Streams** — profundidad (mapa de color) y color (RGB) en vivo, con FPS.
- [x] **Nube de puntos** — deproyección por frame, color emparejado, export **PLY**.
- [x] **Escaneo multi-frame** — registro por ICP punto-a-plano + fusión por vóxeles.
- [x] **Malla** — reconstrucción por campo de distancia con signo (MLS) + Surface
      Nets, suavizado y simplificación, export **OBJ / STL / PLY**.
- [x] **Controles** — exposición/ganancia, volumen de escaneo (caja), detección y
      aislamiento del objeto, calibración fina (FOV + escala de profundidad).

Ver [`ROADMAP.md`](ROADMAP.md) para lo que sigue.

## Stack

Todo en **Rust**, sin motor 3D dedicado. Las dependencias (`Cargo.toml`):

| Crate | Versión | Para qué |
|-------|---------|----------|
| `eframe` | 0.29 | Abre y gestiona la **ventana** del SO (bucle de eventos, contexto de render) |
| `egui` | 0.29 | **GUI** immediate-mode: paneles, controles y visores dentro de la ventana |
| `v4l` | 0.14 | Captura desde el escáner vía **V4L2** (profundidad `Y16` + color `MJPG`) |
| `zune-jpeg` | 0.4 | Decodifica el stream `MJPG` del canal de color |
| `crossbeam-channel` | 0.5 | Pasa frames entre el hilo de captura y el de la GUI |
| `log` + `env_logger` | 0.4 / 0.11 | Logging |

En resumen: `eframe` es la ventana real en Linux y `egui` es lo que se pinta
adentro. El visor 3D de la nube de puntos / malla es un **rasterizador por
software** propio (`pointcloud.rs`), sin `wgpu`, `bevy` ni dependencias gráficas
extra.

## Arquitectura

```
src/
  main.rs        Punto de entrada + ventana eframe
  app.rs         GUI egui: visores, controles, escaneo, malla, calibración
  camera.rs      Descubrimiento V4L2 + tipos de frame + decodificación (Y16/MJPG)
  capture.rs     Hilo de captura V4L2 ↔ canales a la GUI (exposición/ganancia)
  pointcloud.rs  Deproyección a nube, color, export PLY y visor por software
  scan.rs        ICP punto-a-plano, normales, fusión por vóxeles (escaneo)
  mesh.rs        Reconstrucción de malla (MLS + Surface Nets), export OBJ/STL/PLY
scripts/
  install-udev.sh   Reglas udev de acceso USB (V4L2)
  fetch-sdk.sh      (Opcional) baja el SDK propietario, solo de referencia
vendor/3DCamera/    SDK oficial de Revopoint — LEGADO/REFERENCIA (no se usa para
src/sdk/            compilar; la captura va por V4L2). Se conserva por si en el
                    futuro se explora la vía con licencia.
```

La captura va por `uvcvideo` (V4L2): el kernel debe tener el módulo cargado para
que aparezcan los nodos `/dev/video*`; solo hace falta permiso de usuario sobre
ellos (lo dan las reglas udev o el grupo `video`).

## Requisitos

- Linux x86-64, **Rust** (cargo) y un compilador de C para enlazar.
- Dependencias de sistema de egui/eframe (X11 o Wayland, OpenGL) y `v4l`.

## Uso

### Rápido: `./orizon3d.sh`

Script en la raíz que cubre el ciclo de desarrollo. Sin argumentos abre un menú
interactivo; también acepta comandos directos:

```sh
./orizon3d.sh setup    # dependencias de sistema + compila (release)
./orizon3d.sh udev     # instala reglas udev de acceso USB (usa sudo, una vez)
./orizon3d.sh start    # ejecuta la versión release
./orizon3d.sh run      # ejecuta en debug
./orizon3d.sh build    # compila release
./orizon3d.sh test     # tests unitarios
./orizon3d.sh check    # fmt + clippy + tests (estilo CI)
./orizon3d.sh doctor   # diagnostica entorno y detecta el escáner
./orizon3d.sh status   # versiones, rama y commit
./orizon3d.sh help     # lista completa de comandos
```

### Manual

```sh
sudo ./scripts/install-udev.sh   # acceso USB (una vez); reconecta el escáner
cargo run --release
```

Con el escáner conectado verás el stream de profundidad en vivo. Sin escáner, la
app abre igual y muestra el motivo en la barra de estado; usa **⟳ Reconectar**
tras enchufarlo.

## Solución de problemas

- **No detecta el escáner**: comprueba que `uvcvideo` está cargado y que aparecen
  los nodos (`ls /dev/video*`), y que tienes permiso de lectura sobre ellos
  (instala las reglas udev o añade tu usuario al grupo `video` y reinicia sesión).
  `./orizon3d.sh doctor` lo revisa por ti.
- **`rustup could not choose a version of cargo to run … no default is configured`**:
  ocurre con varios `rustup` instalados (p. ej. el de `pacman` en
  `/usr/lib/rustup/bin` y el de rustup.rs en `~/.cargo/bin`). Soluciónalo con
  `rustup default stable`.

## Licencia

GPL-3.0. El SDK propietario `lib3DCamera.so` (legado, no necesario para compilar)
se redistribuye desde el repositorio open-source de Revopoint
[`Revopoint/3DViewer`](https://github.com/Revopoint/3DViewer) y se baja aparte con
`./scripts/fetch-sdk.sh`.
