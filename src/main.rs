//! RevoScan Linux — visor/escáner para escáneres 3D Revopoint (POP 2 / POP 3).
//!
//! Paso 1: detectar el escáner por USB (vía el SDK oficial lib3DCamera) y
//! mostrar el stream de profundidad en vivo.

mod app;
mod capture;
mod sdk;

use app::RevoApp;

fn main() -> eframe::Result<()> {
    // Por defecto solo warnings (silencia el ruido INFO de zbus/wayland);
    // nuestra app a nivel info. Se puede sobreescribir con RUST_LOG.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,revoscan_linux=info"),
    )
    .init();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([960.0, 720.0])
            .with_title("RevoScan Linux"),
        ..Default::default()
    };

    eframe::run_native(
        "RevoScan Linux",
        options,
        Box::new(|cc| Ok(Box::new(RevoApp::new(cc)))),
    )
}
