# RevoScan Linux

Aplicación de escaneo 3D para escáneres **Revopoint POP 2 / POP 3** en Linux,
escrita en **Rust**. Revopoint no publica RevoScan para Linux, pero sí publica
el SDK de cámara `lib3DCamera` (que sí funciona en Linux). Este proyecto
construye, paso a paso, la aplicación que falta sobre ese SDK.

## Estado

**Paso 1 — Conectar y ver el stream ✅**
- Detecta el escáner por USB mediante el SDK oficial (`lib3DCamera.so`, FFI).
- Conecta con la primera cámara disponible.
- Arranca el stream de profundidad y lo muestra en vivo (mapa de color
  azul→rojo) en una ventana egui, con FPS e info de la cámara (nombre, S/N, FW).

### Hoja de ruta
- [x] **Paso 1**: conectar + ver stream de profundidad
- [ ] **Paso 2**: stream RGB emparejado (`getPairedFrame`, decodificar MJPG/H264)
- [ ] **Paso 3**: nube de puntos por frame + exportar PLY (intrínsecos del SDK)
- [ ] **Paso 4**: registro/alineado de frames (ICP) y fusión
- [ ] **Paso 5**: malla + texturizado + exportar OBJ/STL
- [ ] Controles: exposición, ganancia, HDR, modo disparo, rango de profundidad

## Arquitectura

```
src/
  main.rs       Punto de entrada + ventana eframe
  app.rs        GUI egui: visor, colormap de profundidad, estado/FPS
  capture.rs    Hilo de captura (posee la sesión del SDK) ↔ canales a la GUI
  sdk/
    ffi.rs      Bindings FFI crudos contra lib3DCamera (extern "C")
    mod.rs      Envoltura segura (RAII): Session = sistema → cámara → stream
vendor/3DCamera/
  lib/lib3DCamera.so   SDK oficial de Revopoint (Linux x64)
  include/             Headers C/C++ del SDK (referencia)
  cs_uvc.rules         Reglas udev para acceso USB
scripts/install-udev.sh
```

El driver de cámara habla **UVC sobre libusb** directamente (no usa el módulo
`uvcvideo` del kernel), por eso solo hace falta dar permiso de usuario al nodo
USB vía udev. El patrón de captura (callback NULL + polling con `getFrame`)
replica el que usa el visor oficial de Revopoint (`3DViewer`).

## Requisitos

- Linux x86-64, Rust (cargo) y un toolchain de C para enlazar.
- Dependencias de sistema de egui/eframe (X11 o Wayland, OpenGL).

## Instalación y uso

1. Descarga la librería oficial del SDK (~94 MB, no versionada en el repo):

   ```sh
   ./scripts/fetch-sdk.sh
   ```

2. Instala las reglas udev (una sola vez) y reconecta el escáner:

   ```sh
   sudo ./scripts/install-udev.sh
   ```

3. Compila y ejecuta:

   ```sh
   cargo run --release
   ```

Con el escáner conectado verás el stream de profundidad en vivo. Sin escáner,
la app abre igualmente y muestra el motivo en la barra de estado; usa
**⟳ Reconectar** tras enchufarlo.

## Notas

- El binario embebe un `RUNPATH` a `vendor/3DCamera/lib`, así que no necesitas
  `LD_LIBRARY_PATH`.
- `lib3DCamera.so` es un binario propietario de Revopoint redistribuido desde su
  repositorio open-source [`Revopoint/3DViewer`](https://github.com/Revopoint/3DViewer) (GPL-3.0).
