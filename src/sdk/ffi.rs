//! Bindings FFI crudas contra la librería oficial `lib3DCamera.so` de Revopoint.
//!
//! Las firmas se corresponden con los headers en C de `vendor/3DCamera/include/h/`.
//! Toda la API está declarada `extern "C"`, por lo que los símbolos no están
//! "mangled" y el layout de structs es el del ABI de C (`repr(C)`).
#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::os::raw::{c_char, c_int, c_void};

// --- Tipos opacos (handles del SDK) -------------------------------------
#[repr(C)]
pub struct CSystem {
    _private: [u8; 0],
}
#[repr(C)]
pub struct CCamera {
    _private: [u8; 0],
}
#[repr(C)]
pub struct CStream {
    _private: [u8; 0],
}
#[repr(C)]
pub struct CFrame {
    _private: [u8; 0],
}

// --- Enums (representados como i32 en el ABI de C) ----------------------
pub type ERROR_CODE = c_int;
pub const SUCCESS: ERROR_CODE = 0;
pub const ERROR_DEVICE_NOT_CONNECT: ERROR_CODE = 3;
pub const ERROR_FRAME_TIMEOUT: ERROR_CODE = 7;

pub type STREAM_TYPE = c_int;
pub const STREAM_TYPE_DEPTH: STREAM_TYPE = 0;
pub const STREAM_TYPE_RGB: STREAM_TYPE = 1;

pub type STREAM_FORMAT = c_int;
pub const STREAM_FORMAT_MJPG: STREAM_FORMAT = 0x00;
pub const STREAM_FORMAT_RGB8: STREAM_FORMAT = 0x01;
pub const STREAM_FORMAT_Z16: STREAM_FORMAT = 0x02;
pub const STREAM_FORMAT_Z16Y8Y8: STREAM_FORMAT = 0x03;
pub const STREAM_FORMAT_PAIR: STREAM_FORMAT = 0x04;
pub const STREAM_FORMAT_H264: STREAM_FORMAT = 0x05;

pub type FRAME_DATA_FORMAT = c_int;
pub const FRAME_DATA_FORMAT_Z16: FRAME_DATA_FORMAT = 0x00;
pub const FRAME_DATA_FORMAT_IR_LEFT: FRAME_DATA_FORMAT = 0x01;
pub const FRAME_DATA_FORMAT_IR_RIGHT: FRAME_DATA_FORMAT = 0x02;

// --- Structs por valor --------------------------------------------------
/// `StreamInfo` de `Types.hpp`: { STREAM_FORMAT format; int width; int height; float fps; }
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StreamInfo {
    pub format: STREAM_FORMAT,
    pub width: c_int,
    pub height: c_int,
    pub fps: f32,
}

/// `Intrinsics` de `Types.hpp`: resolución de calibración + matriz 3x3.
/// Los campos `zero*`/`one22` son los ceros/uno de la matriz; solo usamos
/// fx, fy, cx, cy.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Intrinsics {
    pub width: i16,
    pub height: i16,
    pub fx: f32,
    pub zero01: f32,
    pub cx: f32,
    pub zeor10: f32,
    pub fy: f32,
    pub cy: f32,
    pub zeor20: f32,
    pub zero21: f32,
    pub one22: f32,
}

/// `Extrinsics` de `Types.hpp`: rotación 3x3 (column-major) + traslación.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Extrinsics {
    pub rotation: [f32; 9],
    pub translation: [f32; 3],
}

/// `CameraInfo` de `Types.hpp`: cinco buffers de char de tamaño fijo.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CameraInfo {
    pub name: [c_char; 32],
    pub serial: [c_char; 32],
    pub unique_id: [c_char; 32],
    pub firmware_version: [c_char; 32],
    pub algorithm_version: [c_char; 32],
}

/// Callback de frames. En el Paso 1 usamos polling, así que lo pasamos como `None`.
pub type CFrameCallback = Option<extern "C" fn(frame: *mut CFrame, user: *mut c_void)>;

#[link(name = "3DCamera")]
extern "C" {
    // -- system.h --
    pub fn createSystem() -> *mut CSystem;
    pub fn deleteSystem(sys: *mut CSystem);
    pub fn systemCreateCameraInfoList(sys: *const CSystem, count: *mut c_int) -> *mut CameraInfo;
    pub fn systemDeleteCameraInfoList(list: *mut CameraInfo);
    pub fn systemConnectCamera(sys: *mut CSystem, info: *mut CameraInfo) -> *mut CCamera;
    pub fn systemDisconnectCamera(sys: *mut CSystem, camera: *mut CCamera);

    // -- camera.h --
    pub fn cameraCreateStreamInfoList(
        camera: *const CCamera,
        type_: STREAM_TYPE,
        count: *mut c_int,
    ) -> *mut StreamInfo;
    pub fn cameraDeleteStreamInfoList(infolist: *mut StreamInfo);
    pub fn cameraStartStream(
        camera: *const CCamera,
        type_: STREAM_TYPE,
        info: StreamInfo,
        on_frame: CFrameCallback,
        user: *mut c_void,
    ) -> *mut CStream;
    pub fn cameraStopStream(sp: *mut CStream);
    // En el header es `CFrame*& frame`; en el ABI de C equivale a `CFrame**`.
    pub fn cameraGetFrame(sp: *mut CStream, frame: *mut *mut CFrame, timeout_ms: c_int)
        -> ERROR_CODE;
    pub fn cameraGetPairedFrame(
        sp: *mut CStream,
        depth_frame: *mut *mut CFrame,
        rgb_frame: *mut *mut CFrame,
        timeout_ms: c_int,
    ) -> ERROR_CODE;
    pub fn cameraReleaseFrame(sp: *mut CStream, frame: *mut CFrame) -> ERROR_CODE;
    pub fn cameraGetStreamIntrinsics(
        device: *const CCamera,
        type_: STREAM_TYPE,
        intr: *mut Intrinsics,
    ) -> ERROR_CODE;
    pub fn cameraGetStreamExtrinsics(device: *const CCamera, extr: *mut Extrinsics) -> ERROR_CODE;

    // -- frame.h --
    pub fn frameGetTimestamp(frame: *const CFrame) -> f64;
    pub fn frameGetData(frame: *const CFrame) -> *const c_void;
    pub fn frameGetDataByFormat(frame: *const CFrame, format: FRAME_DATA_FORMAT) -> *const c_void;
    pub fn frameGetDataSize(frame: *const CFrame) -> c_int;
    pub fn frameGetWidth(frame: *const CFrame) -> c_int;
    pub fn frameGetHeight(frame: *const CFrame) -> c_int;
    pub fn frameGetFormat(frame: *const CFrame) -> STREAM_FORMAT;
}

// --- Funciones de configuración del SDK (C++ en namespace `cs`) ---------
//
// Estas NO están en la C API de `h/`, sino en `hpp/System.hpp`, así que sus
// símbolos están "mangled". Las enlazamos por su nombre mangled exacto (sacado
// con `nm -D lib3DCamera.so`). Sirven para activar el log interno del SDK
// (clave para diagnosticar fallos de conexión) y forzar el backend libuvc.
#[link(name = "3DCamera")]
extern "C" {
    /// `cs::setLogSavePath(const char*)` — ruta del fichero de log del SDK.
    #[link_name = "_ZN2cs14setLogSavePathEPKc"]
    pub fn setLogSavePath(path: *const c_char);

    /// `cs::enableLoging(bool)` — activa/desactiva el log interno (sic, "Loging").
    #[link_name = "_ZN2cs12enableLogingEb"]
    pub fn enableLoging(enable: bool);

    /// `cs::setSdkEnableLibuvc(bool)` — fuerza el backend libuvc/libusb del SDK.
    #[link_name = "_ZN2cs18setSdkEnableLibuvcEb"]
    pub fn setSdkEnableLibuvc(enable: bool);
}
