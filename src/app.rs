//! Interfaz gráfica (egui/eframe): visor de streams + nube de puntos.

use std::path::PathBuf;
use std::time::Instant;

use eframe::egui;

use crate::capture::{Capture, Status};
use crate::pointcloud::{self, CloudParams, OrbitCamera, PointCloud};
use crate::sdk::{CameraDescription, Frames};

pub struct RevoApp {
    capture: Option<Capture>,
    depth_tex: Option<egui::TextureHandle>,
    depth_size: [usize; 2],
    rgb_tex: Option<egui::TextureHandle>,
    rgb_size: [usize; 2],
    status: String,
    info: Option<CameraDescription>,
    stream_desc: String,

    // Calibración + último frame, para generar la nube.
    params: Option<CloudParams>,
    last_frames: Option<Frames>,

    // Vista 3D.
    show_3d: bool,
    cloud: Option<PointCloud>,
    cloud_dirty: bool,
    cloud_tex: Option<egui::TextureHandle>,
    camera: OrbitCamera,

    // Guardado.
    save_msg: String,

    // FPS de visualización.
    frame_count: u32,
    last_fps_instant: Instant,
    fps: f32,
}

impl RevoApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = RevoApp {
            capture: None,
            depth_tex: None,
            depth_size: [0, 0],
            rgb_tex: None,
            rgb_size: [0, 0],
            status: "Inicializando…".to_owned(),
            info: None,
            stream_desc: String::new(),
            params: None,
            last_frames: None,
            show_3d: false,
            cloud: None,
            cloud_dirty: false,
            cloud_tex: None,
            camera: OrbitCamera::default(),
            save_msg: String::new(),
            frame_count: 0,
            last_fps_instant: Instant::now(),
            fps: 0.0,
        };
        app.connect(&cc.egui_ctx);
        app
    }

    fn connect(&mut self, ctx: &egui::Context) {
        self.capture = None;
        self.depth_tex = None;
        self.rgb_tex = None;
        self.info = None;
        self.params = None;
        self.last_frames = None;
        self.cloud = None;
        self.stream_desc.clear();
        self.status = "Conectando…".to_owned();

        let ctx_clone = ctx.clone();
        self.capture = Some(Capture::start(move || ctx_clone.request_repaint()));
    }

    fn drain_status(&mut self) {
        let Some(cap) = &self.capture else { return };
        while let Ok(s) = cap.status.try_recv() {
            match s {
                Status::Connecting => self.status = "Conectando con el escáner…".to_owned(),
                Status::Streaming {
                    info,
                    depth,
                    rgb,
                    params,
                } => {
                    self.status = "Transmitiendo".to_owned();
                    let mut d = format!(
                        "Profundidad {}×{} @ {:.0} fps",
                        depth.width, depth.height, depth.fps
                    );
                    match rgb {
                        Some(r) => {
                            let fmt = if r.format == 1 { "RGB8" } else { "MJPG" };
                            d.push_str(&format!("   ·   RGB {}×{} ({})", r.width, r.height, fmt));
                        }
                        None => d.push_str("   ·   sin RGB"),
                    }
                    self.stream_desc = d;
                    self.info = Some(info);
                    self.params = Some(params);
                }
                Status::Error(e) => self.status = format!("Error: {e}"),
                Status::Stopped => self.status = "Detenido".to_owned(),
            }
        }
    }

    fn drain_frames(&mut self, ctx: &egui::Context) {
        let Some(cap) = &self.capture else { return };
        let mut latest: Option<Frames> = None;
        while let Ok(f) = cap.frames.try_recv() {
            latest = Some(f);
        }
        if let Some(frames) = latest {
            self.update_depth(ctx, &frames);
            if let Some(rgb) = &frames.rgb {
                self.update_rgb(ctx, rgb.width as usize, rgb.height as usize, &rgb.rgb);
            }
            self.last_frames = Some(frames);
            self.cloud_dirty = true;
            self.tick_fps();
        }
    }

    fn tick_fps(&mut self) {
        self.frame_count += 1;
        let elapsed = self.last_fps_instant.elapsed().as_secs_f32();
        if elapsed >= 0.5 {
            self.fps = self.frame_count as f32 / elapsed;
            self.frame_count = 0;
            self.last_fps_instant = Instant::now();
        }
    }

    fn update_depth(&mut self, ctx: &egui::Context, frames: &Frames) {
        let f = &frames.depth;
        let (w, h) = (f.width as usize, f.height as usize);
        if w == 0 || h == 0 || f.depth.len() < w * h {
            return;
        }
        let image = colorize_depth(&f.depth, w, h);
        self.depth_size = [w, h];
        match &mut self.depth_tex {
            Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.depth_tex =
                    Some(ctx.load_texture("depth", image, egui::TextureOptions::LINEAR));
            }
        }
    }

    fn update_rgb(&mut self, ctx: &egui::Context, w: usize, h: usize, rgb: &[u8]) {
        if w == 0 || h == 0 || rgb.len() < w * h * 3 {
            return;
        }
        let image = egui::ColorImage::from_rgb([w, h], &rgb[..w * h * 3]);
        self.rgb_size = [w, h];
        match &mut self.rgb_tex {
            Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.rgb_tex = Some(ctx.load_texture("rgb", image, egui::TextureOptions::LINEAR));
            }
        }
    }

    /// Genera (o regenera) la nube a partir del último frame.
    fn rebuild_cloud(&mut self) {
        let (Some(params), Some(frames)) = (&self.params, &self.last_frames) else {
            return;
        };
        let cloud = PointCloud::generate(&frames.depth, frames.rgb.as_ref(), params);
        self.cloud = Some(cloud);
        self.cloud_dirty = false;
    }

    /// Guarda la nube del último frame como PLY en `captures/`.
    fn save_ply(&mut self) {
        if self.params.is_none() || self.last_frames.is_none() {
            self.save_msg = "No hay frame para guardar todavía.".to_owned();
            return;
        }
        self.rebuild_cloud();
        let Some(cloud) = &self.cloud else { return };
        if cloud.points.is_empty() {
            self.save_msg = "La nube está vacía (sin datos de profundidad).".to_owned();
            return;
        }

        let dir = PathBuf::from("captures");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.save_msg = format!("No se pudo crear captures/: {e}");
            return;
        }
        // Buscar el primer nombre libre.
        let mut path = dir.join("cloud_000.ply");
        for n in 0..1000 {
            let candidate = dir.join(format!("cloud_{n:03}.ply"));
            if !candidate.exists() {
                path = candidate;
                break;
            }
        }
        match cloud.export_ply(&path) {
            Ok(()) => {
                self.save_msg = format!(
                    "Guardado {} ({} puntos{})",
                    path.display(),
                    cloud.points.len(),
                    if cloud.has_color { ", con color" } else { "" }
                );
            }
            Err(e) => self.save_msg = format!("Error al guardar: {e}"),
        }
    }
}

impl eframe::App for RevoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_status();
        self.drain_frames(ctx);

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("RevoScan Linux");
                ui.separator();
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⟳ Reconectar").clicked() {
                        self.connect(ctx);
                    }
                    if self.fps > 0.0 {
                        ui.label(format!("{:.1} fps", self.fps));
                    }
                });
            });
            if let Some(info) = &self.info {
                ui.horizontal(|ui| {
                    ui.label(format!("📷 {}", info.name));
                    ui.separator();
                    ui.label(format!("S/N: {}", info.serial));
                    ui.separator();
                    ui.label(format!("FW: {}", info.firmware));
                });
                if !self.stream_desc.is_empty() {
                    ui.label(&self.stream_desc);
                }
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.show_3d, "Vista 3D (nube)");
                    if ui.button("💾 Guardar PLY").clicked() {
                        self.save_ply();
                    }
                    if self.show_3d {
                        if ui.button("Reset vista").clicked() {
                            self.camera = OrbitCamera::default();
                        }
                        ui.label("· arrastra para rotar, rueda para zoom");
                    }
                });
                if !self.save_msg.is_empty() {
                    ui.label(&self.save_msg);
                }
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.show_3d {
                self.show_cloud_view(ui, ctx);
            } else {
                self.show_2d_view(ui);
            }
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

impl RevoApp {
    fn show_2d_view(&mut self, ui: &mut egui::Ui) {
        let has_depth = self.depth_tex.is_some();
        let has_rgb = self.rgb_tex.is_some();

        if !has_depth && !has_rgb {
            ui.centered_and_justified(|ui| {
                ui.label("Esperando frames…\n\nConecta el escáner Revopoint por USB.");
            });
            return;
        }
        if has_rgb {
            ui.columns(2, |cols| {
                cols[0].vertical_centered(|ui| {
                    ui.label("Profundidad");
                    image_fit(ui, self.depth_tex.as_ref(), self.depth_size);
                });
                cols[1].vertical_centered(|ui| {
                    ui.label("RGB");
                    image_fit(ui, self.rgb_tex.as_ref(), self.rgb_size);
                });
            });
        } else {
            ui.centered_and_justified(|ui| {
                image_fit(ui, self.depth_tex.as_ref(), self.depth_size);
            });
        }
    }

    fn show_cloud_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.cloud_dirty || self.cloud.is_none() {
            self.rebuild_cloud();
        }
        let Some(cloud) = &self.cloud else {
            ui.centered_and_justified(|ui| ui.label("Sin nube todavía."));
            return;
        };
        if cloud.points.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Nube vacía: aún no llegan datos de profundidad válidos.")
            });
            return;
        }

        // Área interactiva.
        let avail = ui.available_size();
        let (rect, resp) = ui.allocate_exact_size(avail, egui::Sense::drag());

        if resp.dragged() {
            let d = resp.drag_delta();
            self.camera.yaw -= d.x * 0.01;
            self.camera.pitch = (self.camera.pitch + d.y * 0.01).clamp(-1.5, 1.5);
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 {
                self.camera.zoom = (self.camera.zoom * (1.0 + scroll * 0.0015)).clamp(0.1, 10.0);
            }
        }

        // Rasterizar a un buffer del tamaño del área (limitado).
        let w = (rect.width() as usize).clamp(16, 1280);
        let h = (rect.height() as usize).clamp(16, 1024);
        let buf = pointcloud::render(cloud, self.camera, w, h);
        let image = egui::ColorImage::from_rgb([w, h], &buf);
        match &mut self.cloud_tex {
            Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.cloud_tex =
                    Some(ctx.load_texture("cloud", image, egui::TextureOptions::LINEAR));
            }
        }
        if let Some(tex) = &self.cloud_tex {
            ui.painter().image(
                tex.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }
}

/// Dibuja una textura encajada en el espacio disponible, manteniendo el aspect.
fn image_fit(ui: &mut egui::Ui, tex: Option<&egui::TextureHandle>, size: [usize; 2]) {
    let Some(tex) = tex else { return };
    let [tw, th] = size;
    if tw == 0 || th == 0 {
        return;
    }
    let avail = ui.available_size();
    let aspect = tw as f32 / th as f32;
    let mut s = egui::vec2(avail.x, avail.x / aspect);
    if s.y > avail.y {
        s = egui::vec2(avail.y * aspect, avail.y);
    }
    ui.image((tex.id(), s));
}

/// Convierte un mapa de profundidad Z16 en una imagen coloreada (rojo cerca,
/// azul lejos). Los píxeles con valor 0 (sin dato) se pintan en negro.
fn colorize_depth(depth: &[u16], w: usize, h: usize) -> egui::ColorImage {
    let (mut min, mut max) = (u16::MAX, u16::MIN);
    for &v in depth.iter() {
        if v != 0 {
            min = min.min(v);
            max = max.max(v);
        }
    }
    let span = if max > min { (max - min) as f32 } else { 1.0 };

    let mut pixels = Vec::with_capacity(w * h);
    for &v in depth.iter().take(w * h) {
        if v == 0 {
            pixels.push(egui::Color32::BLACK);
        } else {
            let t = 1.0 - ((v - min) as f32 / span).clamp(0.0, 1.0);
            let (r, g, b) = hue_ramp(t);
            pixels.push(egui::Color32::from_rgb(r, g, b));
        }
    }
    egui::ColorImage {
        size: [w, h],
        pixels,
    }
}

/// Rampa tipo "jet": t=0 azul → cian → verde → amarillo → t=1 rojo.
fn hue_ramp(t: f32) -> (u8, u8, u8) {
    let hue = (1.0 - t.clamp(0.0, 1.0)) * 240.0;
    hsv_to_rgb(hue, 1.0, 1.0)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}
