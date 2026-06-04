//! Envoltura segura sobre el SDK oficial `lib3DCamera.so`.
//!
//! Toda una sesión (sistema → cámara → streams) vive dentro de un único hilo
//! de captura, por lo que no hace falta compartir los punteros crudos entre
//! hilos. `Session` libera todos los recursos automáticamente al destruirse.

pub mod ffi;

use std::ffi::CString;
use std::os::raw::{c_char, c_int};
use std::path::PathBuf;
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

/// Un frame RGB ya decodificado a RGB8 entrelazado (R,G,B por píxel).
pub struct RgbFrame {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// Conjunto de frames emparejados que entrega la captura a la GUI.
pub struct Frames {
    pub depth: DepthFrame,
    pub rgb: Option<RgbFrame>,
}

fn cstr_to_string(buf: &[c_char]) -> String {
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len()) };
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Sesión completa con un escáner: sistema + cámara + streams (profundidad y,
/// opcionalmente, RGB).
pub struct Session {
    sys: *mut ffi::CSystem,
    camera: *mut ffi::CCamera,
    depth_stream: *mut ffi::CStream,
    /// NULL si la cámara no tiene sensor RGB (p. ej. MINI sin RGB).
    rgb_stream: *mut ffi::CStream,
    pub info: CameraDescription,
    pub depth_info: ffi::StreamInfo,
    pub rgb_info: Option<ffi::StreamInfo>,
    pub depth_intr: ffi::Intrinsics,
    pub rgb_intr: Option<ffi::Intrinsics>,
    pub extrinsics: ffi::Extrinsics,
    /// Escala de profundidad: profundidad_mm = depth_scale · valor. Para la
    /// serie POP el valor por defecto del SDK es 0.1 (igual que 3DViewer).
    pub depth_scale: f32,
}

impl Session {
    /// Ruta del log interno del SDK. Lo activamos para poder diagnosticar
    /// fallos de conexión (el SDK escribe ahí el error real de libusb).
    pub fn sdk_log_path() -> PathBuf {
        PathBuf::from("captures").join("sdk.log")
    }

    /// Configura el log del SDK. El SDK ya escribe sus trazas por stdout, así
    /// que el log a fichero es OPCIONAL: solo se activa con `REVOSCAN_SDK_LOG=1`.
    ///
    /// Importante: el SDK RETIENE el puntero de la ruta y lo usa/libera más
    /// tarde (durante connect), así que la cadena debe vivir para siempre; la
    /// fugamos a propósito con `into_raw()`. Pasar un `CString` temporal aquí
    /// provoca un «double free» al conectar.
    unsafe fn configure_sdk() {
        if std::env::var_os("REVOSCAN_SDK_LOG").is_none() {
            return;
        }
        let _ = std::fs::create_dir_all("captures");
        if let Ok(path) = CString::new(Self::sdk_log_path().to_string_lossy().as_bytes()) {
            ffi::setLogSavePath(path.into_raw()); // fuga intencionada: el SDK lo retiene
            ffi::enableLoging(true);
        }
    }

    /// Crea el sistema, enumera escáneres, conecta el primero y arranca el
    /// stream de profundidad (y el RGB si está disponible).
    pub fn open() -> Result<Session> {
        unsafe {
            Self::configure_sdk();
            let sys = ffi::createSystem();
            if sys.is_null() {
                return Err(SdkError("createSystem() devolvió NULL".into()));
            }

            // 1) Esperar a que la enumeración (asíncrona, vía el hilo de sondeo
            //    libuvc del SDK) registre una cámara con un serial válido. Si
            //    conectamos demasiado pronto, la instancia interna aún no existe
            //    y connect falla. Sondeamos hasta ~6 s.
            let mut chosen: Option<ffi::CameraInfo> = None;
            for _ in 0..30 {
                let mut count: c_int = 0;
                let list = ffi::systemCreateCameraInfoList(sys, &mut count);
                if !list.is_null() && count > 0 {
                    let infos = std::slice::from_raw_parts(list, count as usize);
                    // Preferimos una con serial no vacío (detección completa);
                    // si aún no lo hay, guardamos la primera como reserva.
                    let pick = infos
                        .iter()
                        .find(|c| !cstr_to_string(&c.serial).trim().is_empty())
                        .copied()
                        .unwrap_or(infos[0]);
                    let has_serial = !cstr_to_string(&pick.serial).trim().is_empty();
                    chosen = Some(pick);
                    ffi::systemDeleteCameraInfoList(list);
                    if has_serial {
                        break;
                    }
                } else if !list.is_null() {
                    ffi::systemDeleteCameraInfoList(list);
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }

            let Some(mut chosen) = chosen else {
                ffi::deleteSystem(sys);
                return Err(SdkError(
                    "No se detectó ningún escáner Revopoint. \
                     Conéctalo por USB y revisa las reglas udev (ver README)."
                        .into(),
                ));
            };

            let info = CameraDescription {
                name: cstr_to_string(&chosen.name),
                serial: cstr_to_string(&chosen.serial),
                firmware: cstr_to_string(&chosen.firmware_version),
            };

            // 2) Conectar, con algún reintento por si la instancia tarda un poco
            //    más en quedar lista tras aparecer el serial.
            let mut camera: *mut ffi::CCamera = ptr::null_mut();
            for attempt in 0..4 {
                camera = ffi::systemConnectCamera(sys, &mut chosen);
                if !camera.is_null() {
                    log::info!(
                        "Conectado a «{}» (serial {}) en el intento {}",
                        info.name,
                        info.serial,
                        attempt + 1
                    );
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            if camera.is_null() {
                ffi::deleteSystem(sys);
                return Err(SdkError(format!(
                    "Se detectó la cámara «{}» (serial {}) pero no se pudo conectar \
                     tras varios intentos. Trazas del SDK por consola; o ejecuta con \
                     REVOSCAN_SDK_LOG=1 (→ {}).",
                    info.name,
                    info.serial,
                    Self::sdk_log_path().display()
                )));
            }

            // 3) Arrancar el stream de profundidad.
            let depth_info = match Self::pick_depth_stream(camera) {
                Some(si) => si,
                None => {
                    ffi::systemDisconnectCamera(sys, camera);
                    ffi::deleteSystem(sys);
                    return Err(SdkError(
                        "La cámara no expone ningún stream de profundidad".into(),
                    ));
                }
            };
            let depth_stream = ffi::cameraStartStream(
                camera,
                ffi::STREAM_TYPE_DEPTH,
                depth_info,
                None,
                ptr::null_mut(),
            );
            if depth_stream.is_null() {
                ffi::systemDisconnectCamera(sys, camera);
                ffi::deleteSystem(sys);
                return Err(SdkError("cameraStartStream(DEPTH) falló".into()));
            }

            // 4) Arrancar el stream RGB si hay un formato que sepamos decodificar.
            let rgb_info = Self::pick_rgb_stream(camera);
            let mut rgb_stream: *mut ffi::CStream = ptr::null_mut();
            if let Some(rinfo) = rgb_info {
                rgb_stream = ffi::cameraStartStream(
                    camera,
                    ffi::STREAM_TYPE_RGB,
                    rinfo,
                    None,
                    ptr::null_mut(),
                );
                if rgb_stream.is_null() {
                    log::warn!("No se pudo arrancar el stream RGB; sigo solo con profundidad");
                }
            }

            // 5) Leer parámetros de calibración (intrínsecos/extrínsecos).
            let mut depth_intr = ffi::Intrinsics::default();
            if ffi::cameraGetStreamIntrinsics(camera, ffi::STREAM_TYPE_DEPTH, &mut depth_intr)
                != ffi::SUCCESS
            {
                log::warn!("No se pudieron leer los intrínsecos de profundidad");
            }
            let rgb_intr = if !rgb_stream.is_null() {
                let mut ri = ffi::Intrinsics::default();
                if ffi::cameraGetStreamIntrinsics(camera, ffi::STREAM_TYPE_RGB, &mut ri)
                    == ffi::SUCCESS
                {
                    Some(ri)
                } else {
                    None
                }
            } else {
                None
            };
            let mut extrinsics = ffi::Extrinsics::default();
            let _ = ffi::cameraGetStreamExtrinsics(camera, &mut extrinsics);

            Ok(Session {
                sys,
                camera,
                depth_stream,
                rgb_stream,
                info,
                depth_info,
                rgb_info: if rgb_stream.is_null() { None } else { rgb_info },
                depth_intr,
                rgb_intr,
                extrinsics,
                depth_scale: 0.1,
            })
        }
    }

    /// Mejor `StreamInfo` de profundidad: preferimos Z16, luego Z16Y8Y8.
    unsafe fn pick_depth_stream(camera: *mut ffi::CCamera) -> Option<ffi::StreamInfo> {
        Self::pick_stream(camera, ffi::STREAM_TYPE_DEPTH, &[
            ffi::STREAM_FORMAT_Z16,
            ffi::STREAM_FORMAT_Z16Y8Y8,
        ])
    }

    /// Mejor `StreamInfo` de RGB: preferimos RGB8 (sin decodificar), luego MJPG.
    /// H264 no se soporta aún, así que si solo hay eso devolvemos `None`.
    unsafe fn pick_rgb_stream(camera: *mut ffi::CCamera) -> Option<ffi::StreamInfo> {
        Self::pick_stream(camera, ffi::STREAM_TYPE_RGB, &[
            ffi::STREAM_FORMAT_RGB8,
            ffi::STREAM_FORMAT_MJPG,
        ])
    }

    /// Devuelve el primer `StreamInfo` cuyo formato esté en `preferred`
    /// (en orden de preferencia). Si ninguno coincide, devuelve `None`.
    unsafe fn pick_stream(
        camera: *mut ffi::CCamera,
        stype: ffi::STREAM_TYPE,
        preferred: &[ffi::STREAM_FORMAT],
    ) -> Option<ffi::StreamInfo> {
        let mut count: c_int = 0;
        let list = ffi::cameraCreateStreamInfoList(camera, stype, &mut count);
        if list.is_null() || count <= 0 {
            if !list.is_null() {
                ffi::cameraDeleteStreamInfoList(list);
            }
            return None;
        }
        let infos = std::slice::from_raw_parts(list, count as usize);

        let mut chosen = None;
        for &fmt in preferred {
            if let Some(found) = infos.iter().find(|i| i.format == fmt) {
                // Entre varias resoluciones del mismo formato, la mayor.
                let best = infos
                    .iter()
                    .filter(|i| i.format == fmt)
                    .max_by_key(|i| i.width * i.height)
                    .unwrap_or(found);
                chosen = Some(*best);
                break;
            }
        }

        ffi::cameraDeleteStreamInfoList(list);
        chosen
    }

    /// Obtiene (por polling) el siguiente conjunto de frames.
    /// `Ok(None)` significa timeout / no hay frame todavía.
    pub fn poll(&mut self, timeout_ms: i32) -> Result<Option<Frames>> {
        unsafe {
            if self.rgb_stream.is_null() {
                // Solo profundidad.
                let mut frame: *mut ffi::CFrame = ptr::null_mut();
                let rc = ffi::cameraGetFrame(self.depth_stream, &mut frame, timeout_ms);
                match self.check_rc(rc, frame, self.depth_stream)? {
                    None => Ok(None),
                    Some(()) => {
                        let depth = self.extract_depth(frame);
                        ffi::cameraReleaseFrame(self.depth_stream, frame);
                        Ok(Some(Frames { depth, rgb: None }))
                    }
                }
            } else {
                // Frames emparejados depth + RGB.
                let mut dframe: *mut ffi::CFrame = ptr::null_mut();
                let mut rframe: *mut ffi::CFrame = ptr::null_mut();
                let rc = ffi::cameraGetPairedFrame(
                    self.depth_stream,
                    &mut dframe,
                    &mut rframe,
                    timeout_ms,
                );
                if rc == ffi::ERROR_FRAME_TIMEOUT {
                    return Ok(None);
                }
                if rc != ffi::SUCCESS {
                    if !dframe.is_null() {
                        ffi::cameraReleaseFrame(self.depth_stream, dframe);
                    }
                    if !rframe.is_null() {
                        ffi::cameraReleaseFrame(self.rgb_stream, rframe);
                    }
                    return Err(SdkError(format!(
                        "cameraGetPairedFrame() error (código {rc})"
                    )));
                }

                let depth = self.extract_depth(dframe);
                let rgb = if rframe.is_null() {
                    None
                } else {
                    extract_rgb(rframe)
                };

                if !dframe.is_null() {
                    ffi::cameraReleaseFrame(self.depth_stream, dframe);
                }
                if !rframe.is_null() {
                    ffi::cameraReleaseFrame(self.rgb_stream, rframe);
                }
                Ok(Some(Frames { depth, rgb }))
            }
        }
    }

    /// Traduce el código de retorno de un get a `Ok(None)` (timeout) /
    /// `Ok(Some(()))` (hay frame) / `Err`. Libera el frame si hubo error.
    unsafe fn check_rc(
        &self,
        rc: ffi::ERROR_CODE,
        frame: *mut ffi::CFrame,
        stream: *mut ffi::CStream,
    ) -> Result<Option<()>> {
        if rc == ffi::ERROR_FRAME_TIMEOUT || (rc == ffi::SUCCESS && frame.is_null()) {
            return Ok(None);
        }
        if rc != ffi::SUCCESS {
            if !frame.is_null() {
                ffi::cameraReleaseFrame(stream, frame);
            }
            return Err(SdkError(format!("cameraGetFrame() error (código {rc})")));
        }
        Ok(Some(()))
    }

    unsafe fn extract_depth(&self, frame: *mut ffi::CFrame) -> DepthFrame {
        let width = ffi::frameGetWidth(frame).max(0) as u32;
        let height = ffi::frameGetHeight(frame).max(0) as u32;
        let timestamp_ms = ffi::frameGetTimestamp(frame);

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

        DepthFrame {
            width,
            height,
            depth,
            timestamp_ms,
        }
    }
}

/// Extrae y decodifica un frame RGB a RGB8 entrelazado.
unsafe fn extract_rgb(frame: *mut ffi::CFrame) -> Option<RgbFrame> {
    let width = ffi::frameGetWidth(frame).max(0) as u32;
    let height = ffi::frameGetHeight(frame).max(0) as u32;
    let format = ffi::frameGetFormat(frame);
    let size = ffi::frameGetDataSize(frame).max(0) as usize;
    let data_ptr = ffi::frameGetData(frame);
    if data_ptr.is_null() || size == 0 {
        return None;
    }
    let bytes = std::slice::from_raw_parts(data_ptr as *const u8, size);

    match format {
        // Datos ya en RGB (orden R,G,B), igual que QImage::Format_RGB888.
        ffi::STREAM_FORMAT_RGB8 => {
            let need = (width * height * 3) as usize;
            if width == 0 || height == 0 || bytes.len() < need {
                return None;
            }
            Some(RgbFrame {
                width,
                height,
                rgb: bytes[..need].to_vec(),
            })
        }
        // JPEG comprimido: lo decodificamos a RGB8.
        ffi::STREAM_FORMAT_MJPG => decode_mjpg(bytes),
        other => {
            log::warn!("Formato RGB no soportado todavía: {other} (p. ej. H264)");
            None
        }
    }
}

/// Decodifica un buffer MJPG/JPEG a `RgbFrame` (RGB8) usando zune-jpeg.
fn decode_mjpg(bytes: &[u8]) -> Option<RgbFrame> {
    use zune_jpeg::JpegDecoder;
    let mut decoder = JpegDecoder::new(bytes);
    match decoder.decode() {
        Ok(pixels) => {
            let (w, h) = decoder.dimensions().unwrap_or((0, 0));
            if w == 0 || h == 0 || pixels.len() < w * h * 3 {
                return None;
            }
            Some(RgbFrame {
                width: w as u32,
                height: h as u32,
                rgb: pixels,
            })
        }
        Err(e) => {
            log::warn!("Fallo al decodificar JPEG del stream RGB: {e:?}");
            None
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            if !self.rgb_stream.is_null() {
                ffi::cameraStopStream(self.rgb_stream);
            }
            if !self.depth_stream.is_null() {
                ffi::cameraStopStream(self.depth_stream);
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
