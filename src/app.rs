//! Interfaz gráfica (egui/eframe): visor de streams + nube de puntos.

use std::path::PathBuf;
use std::time::Instant;

use eframe::egui;

use crate::capture::{Capture, DepthControls, Status};
use crate::pointcloud::{self, CloudParams, OrbitCamera, PointCloud};
use crate::scan::ScanSession;
use crate::camera::{CameraDescription, Frames};

/// Ajuste fino de calibración sobre los intrínsecos por FOV real. Persistente en
/// `calibration.txt` para no recalibrar cada vez.
#[derive(Debug, Clone, Copy)]
struct Calib {
    /// Multiplica fx/fy (≈ afinar el FOV). 1.0 = FOV de specs del POP 2.
    fx_scale: f32,
    /// Escala de profundidad (mm por unidad Z16).
    depth_scale: f32,
}

impl Default for Calib {
    fn default() -> Self {
        Calib {
            fx_scale: 1.0,
            depth_scale: crate::camera::DEFAULT_DEPTH_SCALE,
        }
    }
}

impl Calib {
    fn path() -> PathBuf {
        PathBuf::from("calibration.txt")
    }

    fn load() -> Self {
        let mut c = Calib::default();
        if let Ok(s) = std::fs::read_to_string(Self::path()) {
            for line in s.lines() {
                let Some((k, v)) = line.split_once('=') else { continue };
                let Ok(v) = v.trim().parse::<f32>() else { continue };
                if !v.is_finite() {
                    continue;
                }
                match k.trim() {
                    "fx_scale" => c.fx_scale = v,
                    "depth_scale" => c.depth_scale = v,
                    _ => {}
                }
            }
        }
        c
    }

    fn save(&self) -> std::io::Result<()> {
        std::fs::write(
            Self::path(),
            format!("fx_scale={}\ndepth_scale={}\n", self.fx_scale, self.depth_scale),
        )
    }
}

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
    cloud: Option<PointCloud>,
    cloud_dirty: bool,
    cloud_tex: Option<egui::TextureHandle>,
    camera: OrbitCamera,

    // Escaneo multi-frame (Fase 4): sesión de registro + fusión y si está
    // integrando frames activamente.
    scan: Option<ScanSession>,
    scanning: bool,

    // Métricas del último frame para el HUD del visor.
    depth_cm: f32,
    /// Distancia (cm) en el centro de la imagen — a lo que apunta la mira.
    center_cm: f32,
    coverage: f32,
    /// Si la última integración de escaneo se registró bien (para feedback verde/rojo).
    last_track_ok: bool,

    // Calibración (ajuste fino sobre el FOV real).
    calib: Calib,

    // Volumen de escaneo (mm): rango de profundidad (Z) + caja lateral (X/Y).
    clip_min: f32,
    clip_max: f32,
    box_lateral: bool,
    box_cx: f32,
    box_cy: f32,
    box_sx: f32,
    box_sy: f32,

    // Detección/limpieza del objeto.
    clean_noise: bool,
    isolate_object: bool,
    /// Quita píxeles voladores en bordes de profundidad (halo objeto/fondo).
    edge_filter: bool,

    // Exposición/ganancia del sensor de profundidad.
    depth_auto_exposure: bool,
    depth_exposure: i32,
    depth_gain: i32,

    // Malla (Fase 5): modelo reconstruido + vista.
    mesh: Option<crate::mesh::Mesh>,
    view_mesh: bool,
    mesh_voxel: f32,
    mesh_fill: u32,
    mesh_smooth: u32,

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
            cloud: None,
            cloud_dirty: false,
            cloud_tex: None,
            camera: OrbitCamera::default(),
            scan: None,
            scanning: false,
            depth_cm: 0.0,
            center_cm: 0.0,
            coverage: 0.0,
            last_track_ok: true,
            calib: Calib::load(),
            // Volumen ajustado a un objeto cercano (~15–35 cm): corta la pared/
            // fondo más allá de ~45 cm y limita los lados a una ventana de 30 cm,
            // de modo que el objeto sea el grupo conexo mayor y se aísle solo.
            // Se puede ampliar con los sliders de "Volumen de escaneo".
            clip_min: 120.0,
            clip_max: 450.0,
            box_lateral: true,
            box_cx: 0.0,
            box_cy: 0.0,
            box_sx: 300.0,
            box_sy: 300.0,
            clean_noise: true,
            isolate_object: true,
            edge_filter: true,
            depth_auto_exposure: true,
            depth_exposure: 8000,
            depth_gain: 1,
            mesh: None,
            view_mesh: false,
            mesh_voxel: 2.0,
            mesh_fill: 1,
            mesh_smooth: 2,
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
        self.mesh = None;
        self.view_mesh = false;
        self.scan = None;
        self.scanning = false;
        self.stream_desc.clear();
        self.save_msg.clear();
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
                        "Profundidad {}×{} ({})",
                        depth.width,
                        depth.height,
                        depth.fourcc_str()
                    );
                    match rgb {
                        Some(r) => {
                            d.push_str(&format!(
                                "   ·   RGB {}×{} ({})",
                                r.width,
                                r.height,
                                r.fourcc_str()
                            ));
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
            self.update_depth_stats(&frames.depth);
            // Si hay un escaneo en curso, registra y fusiona este frame.
            if self.scanning {
                self.integrate_scan_frame(&frames);
            }
            self.last_frames = Some(frames);
            self.cloud_dirty = true;
            self.tick_fps();
        }
    }

    /// Calcula distancia media (cm) y cobertura (fracción de píxeles válidos)
    /// del frame de profundidad, para guiar al usuario en el visor.
    fn update_depth_stats(&mut self, depth: &crate::camera::DepthFrame) {
        // Escala EFECTIVA (la que el usuario calibra), para que el HUD coincida
        // con la nube.
        let scale = self.calib.depth_scale.max(1e-4);
        let (w, h) = (depth.width as usize, depth.height as usize);

        let mut sum = 0.0f64;
        let mut valid = 0usize;
        for &v in &depth.depth {
            if v != 0 {
                sum += v as f64;
                valid += 1;
            }
        }
        let total = depth.depth.len().max(1);
        self.coverage = valid as f32 / total as f32;
        self.depth_cm = if valid > 0 {
            (sum / valid as f64) as f32 * scale / 10.0
        } else {
            0.0
        };

        // Distancia en una ventana central (lo que apunta la mira) — más útil
        // que el promedio de toda la escena (que incluye el fondo).
        self.center_cm = 0.0;
        if w > 0 && h > 0 && depth.depth.len() >= w * h {
            let (x0, x1) = (w * 2 / 5, (w * 3 / 5).max(w * 2 / 5 + 1));
            let (y0, y1) = (h * 2 / 5, (h * 3 / 5).max(h * 2 / 5 + 1));
            let mut cs = 0.0f64;
            let mut cn = 0usize;
            for y in y0..y1 {
                for x in x0..x1 {
                    let d = depth.depth[y * w + x];
                    if d != 0 {
                        cs += d as f64;
                        cn += 1;
                    }
                }
            }
            if cn > 0 {
                self.center_cm = (cs / cn as f64) as f32 * scale / 10.0;
            }
        }
    }

    /// Calidad de seguimiento del escaneo según el RMSE del último ICP.
    fn tracking_quality(&self) -> (&'static str, egui::Color32) {
        match &self.scan {
            Some(s) if s.stats.registered > 1 => {
                let r = s.stats.last_rmse;
                if r < 4.0 {
                    ("bueno", egui::Color32::from_rgb(80, 200, 120))
                } else if r < 8.0 {
                    ("regular", egui::Color32::from_rgb(230, 200, 80))
                } else {
                    ("débil — mueve más despacio", egui::Color32::from_rgb(230, 110, 90))
                }
            }
            _ => ("—", egui::Color32::GRAY),
        }
    }

    /// Genera la nube del frame, la limpia/aísla, e intégrala en el escaneo.
    /// Limpiar antes de fusionar mantiene el modelo (y el ICP) centrado en el
    /// objeto, no en ruido ni fondo.
    fn integrate_scan_frame(&mut self, frames: &Frames) {
        let Some(params) = self.effective_params() else { return };
        let cloud = PointCloud::generate(&frames.depth, frames.rgb.as_ref(), &params);
        let cloud = if self.clean_noise || self.isolate_object {
            let min_pts = if self.clean_noise { 3 } else { 1 };
            crate::scan::clean_cloud(&cloud, 3.0, min_pts, self.isolate_object)
        } else {
            cloud
        };
        let Some(scan) = &mut self.scan else { return };
        // Resultado del registro → feedback verde/rojo en el visor.
        self.last_track_ok = scan.integrate_frame(&cloud);
    }

    /// Parámetros de nube efectivos: los del stream con el ajuste fino aplicado
    /// (escala XY sobre fx/fy y escala de profundidad de la calibración).
    fn effective_params(&self) -> Option<CloudParams> {
        let mut p = self.params?;
        let s = self.calib.fx_scale.max(0.05);
        p.depth_intr.fx *= s;
        p.depth_intr.fy *= s;
        if let Some(ri) = &mut p.rgb_intr {
            ri.fx *= s;
            ri.fy *= s;
        }
        p.depth_scale = self.calib.depth_scale.max(1e-4);
        // El recorte se expresa como una caja (ROI): Z = rango de profundidad,
        // X/Y = caja lateral si está activa (si no, sin límite lateral).
        p.clip_min_mm = 0.0;
        p.clip_max_mm = 0.0;
        let (xmin, xmax, ymin, ymax) = if self.box_lateral {
            (
                self.box_cx - self.box_sx * 0.5,
                self.box_cx + self.box_sx * 0.5,
                self.box_cy - self.box_sy * 0.5,
                self.box_cy + self.box_sy * 0.5,
            )
        } else {
            (f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY)
        };
        p.roi = Some(crate::pointcloud::Roi {
            min: [xmin, ymin, self.clip_min.max(0.0)],
            max: [xmax, ymax, self.clip_max.max(0.0)],
        });
        p.edge_filter = self.edge_filter;
        Some(p)
    }

    /// Dimensiones (mm) de la caja envolvente de la nube actual, para afinar la
    /// calibración contra un objeto de tamaño conocido.
    fn cloud_bbox_mm(&self) -> Option<[f32; 3]> {
        let c = self.cloud.as_ref()?;
        if c.points.is_empty() {
            return None;
        }
        let mut mn = [f32::MAX; 3];
        let mut mx = [f32::MIN; 3];
        for p in &c.points {
            for (i, v) in [p.x, p.y, p.z].iter().enumerate() {
                mn[i] = mn[i].min(*v);
                mx[i] = mx[i].max(*v);
            }
        }
        Some([mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]])
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

    /// Genera (o regenera) la nube a mostrar: la fusionada del escaneo si hay
    /// sesión, o la del último frame en caso contrario.
    fn rebuild_cloud(&mut self) {
        let base = if let Some(scan) = &self.scan {
            scan.fused_cloud()
        } else {
            let Some(params) = self.effective_params() else {
                self.cloud_dirty = false;
                return;
            };
            let Some(frames) = &self.last_frames else {
                self.cloud_dirty = false;
                return;
            };
            PointCloud::generate(&frames.depth, frames.rgb.as_ref(), &params)
        };

        // Limpieza/detección del objeto: quita ruido y aísla el grupo principal.
        let cloud = if self.clean_noise || self.isolate_object {
            let min_pts = if self.clean_noise { 3 } else { 1 };
            crate::scan::clean_cloud(&base, 3.0, min_pts, self.isolate_object)
        } else {
            base
        };
        self.cloud = Some(cloud);
        self.cloud_dirty = false;
    }

    /// Guarda la nube actual como PLY en `captures/`: la fusionada del escaneo
    /// si hay sesión, o la del último frame en caso contrario.
    fn save_ply(&mut self) {
        let scanning_save = self.scan.is_some();
        if !scanning_save && (self.params.is_none() || self.last_frames.is_none()) {
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
        // Prefijo distinto para nubes fusionadas (escaneo) vs. un solo frame.
        let prefix = if scanning_save { "scan" } else { "cloud" };
        // Buscar el primer nombre libre.
        let mut path = dir.join(format!("{prefix}_000.ply"));
        for n in 0..1000 {
            let candidate = dir.join(format!("{prefix}_{n:03}.ply"));
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

        // Barra superior: identidad, estado y cámara.
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Orizon3D");
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
                    if !info.serial.is_empty() {
                        ui.separator();
                        ui.label(format!("S/N {}", info.serial));
                    }
                    if !self.stream_desc.is_empty() {
                        ui.separator();
                        ui.label(&self.stream_desc);
                    }
                });
            }
        });

        // Barra de escaneo: controles + estadísticas (solo con cámara).
        if self.info.is_some() {
            egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
                ui.add_space(2.0);
                self.scan_toolbar(ui);
                ui.add_space(2.0);
            });
        }

        // Visor de la cámara (el objeto) a la izquierda.
        egui::SidePanel::left("viewfinder")
            .resizable(true)
            .default_width(420.0)
            .min_width(280.0)
            .show(ctx, |ui| {
                self.show_viewfinder(ui);
            });

        // Reconstrucción 3D en vivo (la nube) ocupando el resto.
        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_cloud_view(ui, ctx);
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

impl RevoApp {
    /// Barra de escaneo: controles grandes + estado de la sesión.
    fn scan_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if self.scan.is_none() {
                let b = egui::Button::new(egui::RichText::new("⏺  Iniciar escaneo").strong());
                if ui.add(b).clicked() {
                    self.scan = Some(ScanSession::new());
                    self.scanning = true;
                    self.save_msg.clear();
                }
            } else {
                if self.scanning {
                    if ui.button("⏸  Pausar").clicked() {
                        self.scanning = false;
                    }
                } else if ui.button("▶  Reanudar").clicked() {
                    self.scanning = true;
                }
                if ui.button("🗑  Nuevo").clicked() {
                    self.scan = None;
                    self.scanning = false;
                    self.mesh = None;
                    self.view_mesh = false;
                    self.cloud_dirty = true;
                    self.save_msg.clear();
                }
            }

            ui.separator();
            if ui.button("💾  Guardar PLY").clicked() {
                self.save_ply();
            }
            if ui.button("⟲  Reset vista").clicked() {
                self.camera = OrbitCamera::default();
            }

            // Estado de la sesión, alineado a la derecha.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(scan) = &self.scan {
                    let (q, qc) = self.tracking_quality();
                    ui.colored_label(qc, format!("● seguimiento: {q}"));
                    ui.separator();
                    let s = scan.stats;
                    let mut txt = format!("{} pts · {} frames", scan.point_count(), s.registered);
                    if s.dropped > 0 {
                        txt.push_str(&format!(" · {} descartados", s.dropped));
                    }
                    ui.label(txt);
                } else {
                    ui.weak("listo · pulsa Iniciar y mueve el escáner alrededor del objeto");
                }
            });
        });
        if !self.save_msg.is_empty() {
            ui.label(&self.save_msg);
        }
    }

    /// Visor de la cámara: HUD de guía + profundidad + color.
    fn show_viewfinder(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.heading("Cámara");

        if self.depth_tex.is_none() && self.rgb_tex.is_none() {
            ui.add_space(20.0);
            ui.label("Esperando frames…\n\nApunta el escáner a un objeto a 15–40 cm.");
            return;
        }

        // Todo el contenido va en un área con scroll: así se ve aunque la
        // ventana sea baja y no quepan todas las opciones.
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.show_hud(ui);
                ui.separator();
                self.depth_range_ui(ui);
                ui.separator();

                let w = ui.available_width();
                ui.label("Profundidad");
                let drect = fit_image(ui, self.depth_tex.as_ref(), self.depth_size, w, 320.0);
                // Mira (+) en el centro: marca dónde se mide la distancia.
                if let Some(r) = drect {
                    let c = r.center();
                    let col = egui::Color32::from_rgb(255, 255, 0);
                    let s = 8.0;
                    let painter = ui.painter();
                    painter.line_segment([egui::pos2(c.x - s, c.y), egui::pos2(c.x + s, c.y)], egui::Stroke::new(1.5, col));
                    painter.line_segment([egui::pos2(c.x, c.y - s), egui::pos2(c.x, c.y + s)], egui::Stroke::new(1.5, col));
                }

                if self.rgb_tex.is_some() {
                    ui.add_space(6.0);
                    ui.label("Color");
                    fit_image(ui, self.rgb_tex.as_ref(), self.rgb_size, w, 220.0);
                }

                ui.add_space(8.0);
                egui::CollapsingHeader::new("🧊 Malla (modelo)")
                    .default_open(true)
                    .show(ui, |ui| {
                        self.mesh_ui(ui);
                    });
                egui::CollapsingHeader::new("🔆 Exposición").show(ui, |ui| {
                    self.exposure_ui(ui);
                });
                egui::CollapsingHeader::new("⚙ Calibración").show(ui, |ui| {
                    self.calibration_ui(ui);
                });
                ui.add_space(8.0);
            });
    }

    /// HUD de guía: distancia al objeto y cobertura del frame.
    fn show_hud(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Distancia (mira):");
            if self.center_cm <= 0.0 {
                ui.weak("—");
            } else {
                let (txt, col) = range_hint(self.center_cm);
                ui.colored_label(col, format!("{:.0} cm · {txt}", self.center_cm));
            }
            ui.separator();
            ui.label("Cobertura:");
            let pct = self.coverage * 100.0;
            let col = if self.coverage > 0.15 {
                egui::Color32::from_rgb(80, 200, 120)
            } else if self.coverage > 0.05 {
                egui::Color32::from_rgb(230, 200, 80)
            } else {
                egui::Color32::from_rgb(230, 110, 90)
            };
            ui.colored_label(col, format!("{pct:.0}%"));
        });
    }

    /// Volumen de escaneo (caja delimitadora): rango de profundidad (Z) + recorte
    /// lateral (ancho/alto). Aísla el objeto del fondo Y de los lados (mesa,
    /// pared), como la «bounding box» de RevoScan.
    fn depth_range_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.label("Volumen de escaneo (mm) — recorta el objeto:");
        ui.label("Profundidad (cerca–lejos):");
        changed |= ui
            .add(egui::Slider::new(&mut self.clip_min, 50.0..=1500.0).text("cerca"))
            .changed();
        changed |= ui
            .add(egui::Slider::new(&mut self.clip_max, 50.0..=1500.0).text("lejos"))
            .changed();
        if self.clip_min > self.clip_max - 10.0 {
            self.clip_min = (self.clip_max - 10.0).max(50.0);
        }

        changed |= ui
            .checkbox(&mut self.box_lateral, "Recorte lateral (ancho/alto)")
            .changed();
        if self.box_lateral {
            changed |= ui
                .add(egui::Slider::new(&mut self.box_sx, 40.0..=800.0).text("ancho"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut self.box_sy, 40.0..=800.0).text("alto"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut self.box_cx, -300.0..=300.0).text("centro ←→"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut self.box_cy, -300.0..=300.0).text("centro ↑↓"))
                .changed();
        }

        ui.separator();
        ui.label("Detección del objeto (automática):");
        changed |= ui
            .checkbox(&mut self.clean_noise, "Quitar puntos sueltos (ruido)")
            .changed();
        changed |= ui
            .checkbox(&mut self.isolate_object, "Aislar objeto principal (mayor grupo)")
            .changed();
        changed |= ui
            .checkbox(&mut self.edge_filter, "Limpiar bordes (quitar píxeles voladores)")
            .changed();

        if changed {
            self.cloud_dirty = true;
        }
    }

    /// Envía la exposición/ganancia actuales al hilo de captura.
    fn push_depth_controls(&self) {
        if let Some(cap) = &self.capture {
            cap.set_depth_controls(DepthControls {
                auto_exposure: self.depth_auto_exposure,
                exposure: self.depth_exposure,
                gain: self.depth_gain,
            });
        }
    }

    /// Controles del sensor de profundidad. El sensor IR de la POP solo admite
    /// AUTO-exposición por V4L2 (no expone exposición manual: el control queda
    /// `inactive`), así que la única palanca real es la ganancia.
    fn exposure_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(
                "El sensor de profundidad (IR) solo permite auto-exposición; no \
                 expone exposición manual por V4L2. Única palanca: ganancia.",
            )
            .small()
            .color(egui::Color32::from_gray(160)),
        );
        let changed = ui
            .add(egui::Slider::new(&mut self.depth_gain, 1..=16).text("ganancia"))
            .changed();
        ui.label(
            egui::RichText::new(
                "Recomendado ≤3: más ganancia ilumina pero mete ruido y baja la precisión.",
            )
            .small()
            .color(egui::Color32::from_gray(160)),
        );
        if changed {
            self.push_depth_controls();
        }
    }

    /// Reconstruye la malla desde la nube actual (limpia/fusionada).
    fn generate_mesh(&mut self) {
        self.rebuild_cloud();
        let voxel = self.mesh_voxel.max(1.0);
        let fill = self.mesh_fill;
        let smooth = self.mesh_smooth;
        let result = match &self.cloud {
            Some(c) if !c.points.is_empty() => Some(crate::mesh::reconstruct(c, voxel, fill, smooth)),
            _ => None,
        };
        match result {
            Some(m) if !m.is_empty() => {
                self.save_msg = format!(
                    "Malla: {} vértices · {} triángulos",
                    m.vertices.len(),
                    m.tris.len()
                );
                self.view_mesh = true;
                self.mesh = Some(m);
            }
            Some(_) => {
                self.save_msg = "La malla salió vacía (nube insuficiente).".to_owned();
                self.mesh = None;
            }
            None => self.save_msg = "No hay nube para mallar.".to_owned(),
        }
    }

    /// Exporta la malla a `captures/mesh_NNN.<ext>` (obj/stl/ply).
    fn export_mesh(&mut self, ext: &str) {
        if self.mesh.as_ref().map_or(true, |m| m.is_empty()) {
            self.save_msg = "Genera la malla primero.".to_owned();
            return;
        }
        let dir = PathBuf::from("captures");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.save_msg = format!("No se pudo crear captures/: {e}");
            return;
        }
        let mut path = dir.join(format!("mesh_000.{ext}"));
        for n in 0..1000 {
            let c = dir.join(format!("mesh_{n:03}.{ext}"));
            if !c.exists() {
                path = c;
                break;
            }
        }
        let m = self.mesh.as_ref().unwrap();
        let r = match ext {
            "obj" => m.export_obj(&path),
            "stl" => m.export_stl(&path),
            _ => m.export_ply(&path),
        };
        self.save_msg = match r {
            Ok(()) => format!("Guardado {}", path.display()),
            Err(e) => format!("Error al guardar malla: {e}"),
        };
    }

    /// Sección de malla: detalle, relleno de huecos, generar, ver, simplificar
    /// y exportar.
    fn mesh_ui(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::Slider::new(&mut self.mesh_voxel, 1.0..=8.0).text("detalle (mm)"));
        ui.add(egui::Slider::new(&mut self.mesh_smooth, 0..=4).text("suavizado"));
        ui.add(egui::Slider::new(&mut self.mesh_fill, 0..=4).text("rellenar huecos"));
        ui.horizontal(|ui| {
            if ui.button("🧊 Generar malla").clicked() {
                self.generate_mesh();
            }
            if self.mesh.is_some() {
                ui.checkbox(&mut self.view_mesh, "Ver malla");
            }
        });
        let stats = self.mesh.as_ref().map(|m| (m.vertices.len(), m.tris.len()));
        if let Some((nv, nt)) = stats {
            ui.label(format!("{nv} vértices · {nt} triángulos"));
            ui.horizontal(|ui| {
                if ui.button("Simplificar").clicked() {
                    self.decimate_mesh();
                }
                ui.label("·  exportar:");
                if ui.button("OBJ").clicked() {
                    self.export_mesh("obj");
                }
                if ui.button("STL").clicked() {
                    self.export_mesh("stl");
                }
                if ui.button("PLY").clicked() {
                    self.export_mesh("ply");
                }
            });
        } else {
            ui.weak("Genera la malla desde la nube actual.");
        }
    }

    /// Simplifica la malla actual (agrupa vértices a ~1.6× el detalle).
    fn decimate_mesh(&mut self) {
        let cell = (self.mesh_voxel * 1.6).max(1.0);
        if let Some(m) = self.mesh.take() {
            let d = m.decimate(cell);
            self.save_msg = format!(
                "Simplificada: {} vértices · {} triángulos",
                d.vertices.len(),
                d.tris.len()
            );
            self.mesh = Some(d);
        }
    }

    /// Ajuste fino de calibración: escala XY (FOV) y escala de profundidad, con
    /// lectura del tamaño de la nube para cuadrarlo con un objeto real.
    fn calibration_ui(&mut self, ui: &mut egui::Ui) {
        ui.label("Intrínsecos por FOV real del POP 2. Afina con un objeto de tamaño conocido.");
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Escala XY (FOV)");
            changed |= ui
                .add(egui::Slider::new(&mut self.calib.fx_scale, 0.6..=1.6).fixed_decimals(3))
                .changed();
        });
        ui.horizontal(|ui| {
            ui.label("Escala Z (mm/u)");
            changed |= ui
                .add(egui::Slider::new(&mut self.calib.depth_scale, 0.05..=0.20).fixed_decimals(3))
                .changed();
        });
        if changed {
            self.cloud_dirty = true;
        }
        if let Some(d) = self.cloud_bbox_mm() {
            ui.label(format!("Tamaño nube ≈ {:.0} × {:.0} × {:.0} mm", d[0], d[1], d[2]));
        }
        ui.horizontal(|ui| {
            if ui.button("💾 Guardar").clicked() {
                self.save_msg = match self.calib.save() {
                    Ok(()) => "Calibración guardada (calibration.txt)".to_owned(),
                    Err(e) => format!("Error guardando calibración: {e}"),
                };
            }
            if ui.button("Restablecer").clicked() {
                self.calib = Calib::default();
                self.cloud_dirty = true;
            }
        });
    }

    fn show_cloud_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.cloud_dirty || self.cloud.is_none() {
            self.rebuild_cloud();
        }
        let Some(cloud) = &self.cloud else {
            ui.centered_and_justified(|ui| {
                ui.label("Reconstrucción 3D\n\nApunta al objeto; pulsa «Iniciar escaneo».")
            });
            return;
        };
        if cloud.points.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Sin datos de profundidad válidos todavía (acerca el objeto).")
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
        // Durante el escaneo, tinta la nube por calidad de seguimiento:
        // verde = registrando bien, ámbar = regular, rojo = perdido.
        let tint = if self.scanning && self.scan.is_some() {
            if !self.last_track_ok {
                Some([1.0, 0.35, 0.30]) // rojo: no se registró
            } else {
                let rmse = self.scan.as_ref().map(|s| s.stats.last_rmse).unwrap_or(0.0);
                if rmse < 4.0 {
                    Some([0.45, 1.0, 0.55]) // verde: bien
                } else {
                    Some([1.0, 0.85, 0.4]) // ámbar: regular
                }
            }
        } else {
            None
        };

        // En modo malla, rasterizamos el modelo sólido; si no, la nube.
        let buf = match (self.view_mesh, &self.mesh) {
            (true, Some(m)) if !m.is_empty() => crate::mesh::render_mesh(m, self.camera, w, h),
            _ => pointcloud::render(cloud, self.camera, w, h, tint),
        };
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

/// Dibuja una textura ajustada a un ancho/alto máximos, manteniendo el aspecto.
fn fit_image(
    ui: &mut egui::Ui,
    tex: Option<&egui::TextureHandle>,
    size: [usize; 2],
    max_w: f32,
    max_h: f32,
) -> Option<egui::Rect> {
    let tex = tex?;
    let [tw, th] = size;
    if tw == 0 || th == 0 {
        return None;
    }
    let aspect = tw as f32 / th as f32;
    let mut w = max_w;
    let mut h = w / aspect;
    if h > max_h {
        h = max_h;
        w = h * aspect;
    }
    Some(ui.image((tex.id(), egui::vec2(w, h))).rect)
}

/// Texto y color de guía según la distancia (cm). Rango útil de la serie POP
/// ~15–40 cm.
fn range_hint(cm: f32) -> (&'static str, egui::Color32) {
    let green = egui::Color32::from_rgb(80, 200, 120);
    let yellow = egui::Color32::from_rgb(230, 200, 80);
    if cm < 12.0 {
        ("demasiado cerca", yellow)
    } else if cm <= 45.0 {
        ("distancia óptima", green)
    } else {
        ("algo lejos", yellow)
    }
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
