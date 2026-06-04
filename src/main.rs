//! Orizon3D — visor/escáner para escáneres 3D Revopoint (POP 2 / POP 3) en Linux.
//!
//! Detecta el escáner por V4L2 (UVC), muestra profundidad + color en vivo, genera
//! la nube de puntos, registra y fusiona multi-frame (ICP) y reconstruye la malla.

mod app;
mod camera;
mod capture;
mod mesh;
mod pointcloud;
mod scan;

use app::RevoApp;

fn main() -> eframe::Result<()> {
    // Por defecto solo warnings (silencia el ruido INFO de zbus/wayland);
    // nuestra app a nivel info. Se puede sobreescribir con RUST_LOG.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,orizon3d=info"),
    )
    .init();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([960.0, 720.0])
            .with_title("Orizon3D"),
        ..Default::default()
    };

    eframe::run_native(
        "Orizon3D",
        options,
        Box::new(|cc| Ok(Box::new(RevoApp::new(cc)))),
    )
}
