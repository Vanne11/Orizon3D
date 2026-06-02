//! Interfaz gráfica (egui/eframe): visor en vivo del stream de profundidad.

use std::time::Instant;

use eframe::egui;

use crate::capture::{Capture, Status};
use crate::sdk::{CameraDescription, DepthFrame};

pub struct RevoApp {
    capture: Option<Capture>,
    texture: Option<egui::TextureHandle>,
    tex_size: [usize; 2],
    status: String,
    info: Option<CameraDescription>,
    stream_desc: String,
    // Métricas de FPS de visualización.
    frame_count: u32,
    last_fps_instant: Instant,
    fps: f32,
}

impl RevoApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = RevoApp {
            capture: None,
            texture: None,
            tex_size: [0, 0],
            status: "Inicializando…".to_owned(),
            info: None,
            stream_desc: String::new(),
            frame_count: 0,
            last_fps_instant: Instant::now(),
            fps: 0.0,
        };
        app.connect(&cc.egui_ctx);
        app
    }

    /// Arranca (o reinicia) el hilo de captura.
    fn connect(&mut self, ctx: &egui::Context) {
        self.capture = None; // detiene el hilo anterior si existía (Drop)
        self.texture = None;
        self.info = None;
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
                Status::Streaming { info, stream } => {
                    self.status = "Transmitiendo".to_owned();
                    self.stream_desc = format!(
                        "{}×{} @ {:.0} fps (formato {})",
                        stream.width, stream.height, stream.fps, stream.format
                    );
                    self.info = Some(info);
                }
                Status::Error(e) => self.status = format!("Error: {e}"),
                Status::Stopped => self.status = "Detenido".to_owned(),
            }
        }
    }

    fn drain_frames(&mut self, ctx: &egui::Context) {
        let Some(cap) = &self.capture else { return };
        let mut latest: Option<DepthFrame> = None;
        while let Ok(f) = cap.frames.try_recv() {
            latest = Some(f);
        }
        if let Some(frame) = latest {
            self.update_texture(ctx, &frame);
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

    fn update_texture(&mut self, ctx: &egui::Context, frame: &DepthFrame) {
        let (w, h) = (frame.width as usize, frame.height as usize);
        if w == 0 || h == 0 || frame.depth.len() < w * h {
            return;
        }
        let image = colorize_depth(&frame.depth, w, h);
        self.tex_size = [w, h];
        match &mut self.texture {
            Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.texture =
                    Some(ctx.load_texture("depth", image, egui::TextureOptions::LINEAR));
            }
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
                    if !self.stream_desc.is_empty() {
                        ui.separator();
                        ui.label(&self.stream_desc);
                    }
                });
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(tex) = &self.texture {
                // Encajar la imagen manteniendo el aspect ratio.
                let avail = ui.available_size();
                let [tw, th] = self.tex_size;
                let aspect = tw as f32 / th as f32;
                let mut size = egui::vec2(avail.x, avail.x / aspect);
                if size.y > avail.y {
                    size = egui::vec2(avail.y * aspect, avail.y);
                }
                ui.centered_and_justified(|ui| {
                    ui.image((tex.id(), size));
                });
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Esperando frames de profundidad…\n\nConecta el escáner Revopoint por USB.");
                });
            }
        });

        // Refresco continuo mientras transmitimos.
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

/// Convierte un mapa de profundidad Z16 en una imagen coloreada (azul→rojo).
/// Los píxeles con valor 0 (sin dato) se pintan en negro.
fn colorize_depth(depth: &[u16], w: usize, h: usize) -> egui::ColorImage {
    // Rango dinámico sobre los valores válidos (no cero).
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
            // Cerca = rojo, lejos = azul (invertimos t).
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

/// Rampa de color tipo "jet": t=0 azul → cian → verde → amarillo → t=1 rojo.
fn hue_ramp(t: f32) -> (u8, u8, u8) {
    // Hue de 240° (azul) a 0° (rojo) con saturación y valor máximos.
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
