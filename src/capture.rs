//! Hilo de captura: posee la `Session` del SDK y entrega frames a la GUI.
//!
//! Los punteros del SDK no son `Send`, así que toda la sesión se crea, usa y
//! destruye dentro de este mismo hilo. La comunicación con la interfaz se hace
//! por canales (frames y estado).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};

use crate::sdk::{ffi::StreamInfo, CameraDescription, Frames, Session};

/// Estado del pipeline de captura, para mostrarlo en la UI.
#[derive(Debug, Clone)]
pub enum Status {
    Connecting,
    Streaming {
        info: CameraDescription,
        depth: StreamInfo,
        rgb: Option<StreamInfo>,
    },
    Error(String),
    Stopped,
}

/// Maneja el hilo de captura y expone los canales de frames y estado.
pub struct Capture {
    pub frames: Receiver<Frames>,
    pub status: Receiver<Status>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Capture {
    /// Arranca el hilo de captura. Se conecta e inicia los streams dentro del
    /// hilo. `wake` se invoca con cada frame para refrescar la GUI.
    pub fn start<F>(wake: F) -> Self
    where
        F: Fn() + Send + 'static,
    {
        let (frame_tx, frame_rx) = bounded::<Frames>(2);
        let (status_tx, status_rx) = unbounded::<Status>();
        let stop = Arc::new(AtomicBool::new(false));

        let stop_thread = stop.clone();
        let join = std::thread::Builder::new()
            .name("revoscan-capture".into())
            .spawn(move || capture_loop(stop_thread, frame_tx, status_tx, wake))
            .expect("no se pudo crear el hilo de captura");

        Capture {
            frames: frame_rx,
            status: status_rx,
            stop,
            join: Some(join),
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn capture_loop<F>(
    stop: Arc<AtomicBool>,
    frame_tx: Sender<Frames>,
    status_tx: Sender<Status>,
    wake: F,
) where
    F: Fn(),
{
    let _ = status_tx.send(Status::Connecting);

    let mut session = match Session::open() {
        Ok(s) => {
            let _ = status_tx.send(Status::Streaming {
                info: s.info.clone(),
                depth: s.depth_info,
                rgb: s.rgb_info,
            });
            s
        }
        Err(e) => {
            let _ = status_tx.send(Status::Error(e.to_string()));
            return;
        }
    };

    while !stop.load(Ordering::SeqCst) {
        match session.poll(1000) {
            Ok(Some(frames)) => {
                // Buffer acotado: si está lleno descartamos este frame para no
                // acumular latencia (la GUI siempre toma el más reciente).
                let _ = frame_tx.try_send(frames);
                wake();
            }
            Ok(None) => { /* timeout: seguimos esperando */ }
            Err(e) => {
                let _ = status_tx.send(Status::Error(e.to_string()));
                break;
            }
        }
    }

    let _ = status_tx.send(Status::Stopped);
    // `session` se destruye aquí: para los streams, desconecta y libera todo.
}
