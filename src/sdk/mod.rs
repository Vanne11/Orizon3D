//! Envoltura segura sobre el SDK oficial `lib3DCamera.so`.
//!
//! Toda una sesión (sistema → cámara → stream) vive dentro de un único hilo
//! de captura, por lo que no hace falta compartir los punteros crudos entre
//! hilos. `Session` libera todos los recursos automáticamente al destruirse.

pub mod ffi;

use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

/// Error genérico del SDK.
#[derive(Debug, Clone)]
pub struct SdkError(pub String);

impl std::fmt::Display for SdkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for SdkError {}

type Result<T> = std::result::Result<T, SdkError>;

/// Información legible de la cámara conectada.
#[derive(Debug, Clone, Default)]
pub struct CameraDescription {
    pub name: String,
    pub serial: String,
    pub firmware: String,
}

/// Un frame de profundidad ya copiado a memoria propia (independiente del SDK).
pub struct DepthFrame {
    pub width: u32,
    pub height: u32,
    /// Valores Z16: profundidad en mm = `depth_scale * valor`.
    pub depth: Vec<u16>,
    /// Se usará al guardar nubes de puntos con marca de tiempo (Paso 3).
    #[allow(dead_code)]
    pub timestamp_ms: f64,
}

fn cstr_to_string(buf: &[c_char]) -> String {
    // Reinterpretamos como bytes y cortamos en el primer NUL.
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len()) };
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Sesión completa con un escáner: sistema + cámara + stream de profundidad.
pub struct Session {
    sys: *mut ffi::CSystem,
    camera: *mut ffi::CCamera,
    stream: *mut ffi::CStream,
    pub info: CameraDescription,
    pub stream_info: ffi::StreamInfo,
}

impl Session {
    /// Crea el sistema, enumera escáneres, conecta el primero y arranca el
    /// stream de profundidad. Devuelve error si no hay ningún escáner.
    pub fn open() -> Result<Session> {
        unsafe {
            let sys = ffi::createSystem();
            if sys.is_null() {
                return Err(SdkError("createSystem() devolvió NULL".into()));
            }

            // 1) Enumerar cámaras conectadas.
            let mut count: c_int = 0;
            let list = ffi::systemCreateCameraInfoList(sys, &mut count);
            if list.is_null() || count <= 0 {
                if !list.is_null() {
                    ffi::systemDeleteCameraInfoList(list);
                }
                ffi::deleteSystem(sys);
                return Err(SdkError(
                    "No se detectó ningún escáner Revopoint. \
                     Conéctalo por USB y revisa las reglas udev (ver README)."
                        .into(),
                ));
            }

            // 2) Tomar la primera cámara de la lista.
            let infos = std::slice::from_raw_parts(list, count as usize);
            let mut chosen = infos[0]; // copia propia; el SDK casa por serial/uniqueId
            let info = CameraDescription {
                name: cstr_to_string(&chosen.name),
                serial: cstr_to_string(&chosen.serial),
                firmware: cstr_to_string(&chosen.firmware_version),
            };

            let camera = ffi::systemConnectCamera(sys, &mut chosen);
            ffi::systemDeleteCameraInfoList(list);
            if camera.is_null() {
                ffi::deleteSystem(sys);
                return Err(SdkError(format!(
                    "No se pudo conectar con la cámara «{}»",
                    info.name
                )));
            }

            // 3) Elegir un formato de stream de profundidad y arrancarlo.
            let stream_info = match Self::pick_depth_stream(camera) {
                Some(si) => si,
                None => {
                    ffi::systemDisconnectCamera(sys, camera);
                    ffi::deleteSystem(sys);
                    return Err(SdkError(
                        "La cámara no expone ningún stream de profundidad".into(),
                    ));
                }
            };

            // Callback NULL → consumimos los frames por polling (igual que 3DViewer).
            let stream = ffi::cameraStartStream(
                camera,
                ffi::STREAM_TYPE_DEPTH,
                stream_info,
                None,
                ptr::null_mut(),
            );
            if stream.is_null() {
                ffi::systemDisconnectCamera(sys, camera);
                ffi::deleteSystem(sys);
                return Err(SdkError("cameraStartStream() falló".into()));
            }

            Ok(Session {
                sys,
                camera,
                stream,
                info,
                stream_info,
            })
        }
    }

    /// Selecciona el mejor `StreamInfo` de profundidad: preferimos Z16, luego
    /// Z16Y8Y8, y en su defecto el primero disponible.
    unsafe fn pick_depth_stream(camera: *mut ffi::CCamera) -> Option<ffi::StreamInfo> {
        let mut count: c_int = 0;
        let list = ffi::cameraCreateStreamInfoList(camera, ffi::STREAM_TYPE_DEPTH, &mut count);
        if list.is_null() || count <= 0 {
            if !list.is_null() {
                ffi::cameraDeleteStreamInfoList(list);
            }
            return None;
        }
        let infos = std::slice::from_raw_parts(list, count as usize);

        let pick = |fmt: ffi::STREAM_FORMAT| infos.iter().find(|i| i.format == fmt).copied();
        let chosen = pick(ffi::STREAM_FORMAT_Z16)
            .or_else(|| pick(ffi::STREAM_FORMAT_Z16Y8Y8))
            .unwrap_or(infos[0]);

        ffi::cameraDeleteStreamInfoList(list);
        Some(chosen)
    }

    /// Obtiene (por polling) el siguiente frame de profundidad.
    /// `Ok(None)` significa timeout / no hay frame todavía.
    pub fn poll_depth(&mut self, timeout_ms: i32) -> Result<Option<DepthFrame>> {
        unsafe {
            let mut frame: *mut ffi::CFrame = ptr::null_mut();
            let rc = ffi::cameraGetFrame(self.stream, &mut frame, timeout_ms);

            if rc == ffi::ERROR_FRAME_TIMEOUT || (rc == ffi::SUCCESS && frame.is_null()) {
                return Ok(None);
            }
            if rc != ffi::SUCCESS {
                if !frame.is_null() {
                    ffi::cameraReleaseFrame(self.stream, frame);
                }
                return Err(SdkError(format!("cameraGetFrame() error (código {rc})")));
            }

            let width = ffi::frameGetWidth(frame).max(0) as u32;
            let height = ffi::frameGetHeight(frame).max(0) as u32;
            let timestamp_ms = ffi::frameGetTimestamp(frame);

            // El plano de profundidad Z16 es válido tanto para formato Z16 como
            // para el compuesto Z16Y8Y8.
            let mut data_ptr = ffi::frameGetDataByFormat(frame, ffi::FRAME_DATA_FORMAT_Z16);
            if data_ptr.is_null() {
                data_ptr = ffi::frameGetData(frame);
            }

            let mut depth = Vec::new();
            if !data_ptr.is_null() && width > 0 && height > 0 {
                let n = (width * height) as usize;
                let src = std::slice::from_raw_parts(data_ptr as *const u16, n);
                depth.extend_from_slice(src);
            }

            ffi::cameraReleaseFrame(self.stream, frame);

            Ok(Some(DepthFrame {
                width,
                height,
                depth,
                timestamp_ms,
            }))
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            if !self.stream.is_null() {
                ffi::cameraStopStream(self.stream);
            }
            if !self.camera.is_null() {
                ffi::systemDisconnectCamera(self.sys, self.camera);
            }
            if !self.sys.is_null() {
                ffi::deleteSystem(self.sys);
            }
        }
    }
}

// Helpers de conversión de void* sin usar (silencia el warning del import).
#[allow(dead_code)]
fn _unused(_: *mut c_void) {}
