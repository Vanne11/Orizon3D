//! Generación de nube de puntos desde profundidad, exportación a PLY y un
//! visor 3D simple (rasterización por software a una imagen).
//!
//! La deproyección replica la del SDK oficial (`Pointcloud::generatePoint` en
//! `Processing.hpp`): se escalan los intrínsecos a la resolución del frame y
//! se deproyecta cada píxel a coordenadas de cámara (en mm).

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use crate::camera::{DepthFrame, Extrinsics, Intrinsics, RgbFrame};

/// Caja delimitadora (región de interés) en coordenadas de cámara/mundo (mm).
/// Solo se conservan los puntos dentro de la caja: aísla el objeto del entorno.
#[derive(Debug, Clone, Copy)]
pub struct Roi {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// Parámetros de calibración necesarios para generar la nube.
#[derive(Debug, Clone, Copy)]
pub struct CloudParams {
    pub depth_intr: Intrinsics,
    pub rgb_intr: Option<Intrinsics>,
    pub extrinsics: Extrinsics,
    pub depth_scale: f32,
    /// Recorte de profundidad (mm) para descartar fondo/ruido fuera del volumen
    /// de trabajo. `0` = sin límite.
    pub clip_min_mm: f32,
    pub clip_max_mm: f32,
    /// Caja delimitadora 3D opcional (recorta también a los lados).
    pub roi: Option<Roi>,
}

/// Un punto 3D con color opcional.
#[derive(Debug, Clone, Copy)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rgb: [u8; 3],
}

#[derive(Default)]
pub struct PointCloud {
    pub points: Vec<Point>,
    pub has_color: bool,
}

impl PointCloud {
    /// Genera la nube deproyectando el mapa de profundidad. Si hay frame RGB y
    /// parámetros RGB/extrínsecos, colorea cada punto muestreando la imagen RGB.
    pub fn generate(depth: &DepthFrame, rgb: Option<&RgbFrame>, p: &CloudParams) -> PointCloud {
        let w = depth.width as i32;
        let h = depth.height as i32;
        if w <= 0 || h <= 0 || depth.depth.len() < (w * h) as usize {
            return PointCloud::default();
        }

        // Escalar intrínsecos de profundidad a la resolución del frame.
        let id = &p.depth_intr;
        let (sx, sy) = if id.width > 0 && id.height > 0 {
            (w as f32 / id.width as f32, h as f32 / id.height as f32)
        } else {
            (1.0, 1.0)
        };
        let fx = id.fx * sx;
        let fy = id.fy * sy;
        let cx = id.cx * sx;
        let cy = id.cy * sy;
        if fx == 0.0 || fy == 0.0 {
            // Sin intrínsecos válidos no podemos deproyectar.
            return PointCloud::default();
        }

        let want_color = rgb.is_some() && p.rgb_intr.is_some();
        let mut points = Vec::with_capacity((w * h) as usize / 2);

        for v in 0..h {
            for u in 0..w {
                let d = depth.depth[(v * w + u) as usize];
                if d == 0 {
                    continue;
                }
                let z = d as f32 * p.depth_scale;
                if z <= 0.0 {
                    continue;
                }
                // Recorte por volumen de trabajo (descarta fondo/ruido).
                if p.clip_min_mm > 0.0 && z < p.clip_min_mm {
                    continue;
                }
                if p.clip_max_mm > 0.0 && z > p.clip_max_mm {
                    continue;
                }
                let x = (u as f32 - cx) * z / fx;
                // `y_img` sigue la convención de imagen (v hacia abajo); el mundo
                // usa Y hacia arriba, así que invertimos para que el objeto no
                // salga «de cabeza». El color se muestrea con `y_img` (frame de
                // cámara real).
                let y_img = (v as f32 - cy) * z / fy;
                let y = -y_img;

                // Caja delimitadora: descarta lo que quede fuera (también lados).
                if let Some(r) = &p.roi {
                    if x < r.min[0]
                        || x > r.max[0]
                        || y < r.min[1]
                        || y > r.max[1]
                        || z < r.min[2]
                        || z > r.max[2]
                    {
                        continue;
                    }
                }

                let color = if want_color {
                    sample_color(
                        x,
                        y_img,
                        z,
                        rgb.unwrap(),
                        p.rgb_intr.as_ref().unwrap(),
                        &p.extrinsics,
                    )
                } else {
                    // Sin color real: gris según el valor para que se vea algo.
                    [200, 200, 200]
                };

                points.push(Point { x, y, z, rgb: color });
            }
        }

        PointCloud {
            points,
            has_color: want_color,
        }
    }

    /// Exporta la nube a un fichero PLY ASCII (con color si lo hay).
    pub fn export_ply(&self, path: &Path) -> io::Result<()> {
        let f = File::create(path)?;
        let mut out = BufWriter::new(f);

        writeln!(out, "ply")?;
        writeln!(out, "format ascii 1.0")?;
        writeln!(out, "comment generado por Orizon3D")?;
        writeln!(out, "element vertex {}", self.points.len())?;
        writeln!(out, "property float x")?;
        writeln!(out, "property float y")?;
        writeln!(out, "property float z")?;
        if self.has_color {
            writeln!(out, "property uchar red")?;
            writeln!(out, "property uchar green")?;
            writeln!(out, "property uchar blue")?;
        }
        writeln!(out, "end_header")?;

        for pt in &self.points {
            if self.has_color {
                writeln!(
                    out,
                    "{} {} {} {} {} {}",
                    pt.x, pt.y, pt.z, pt.rgb[0], pt.rgb[1], pt.rgb[2]
                )?;
            } else {
                writeln!(out, "{} {} {}", pt.x, pt.y, pt.z)?;
            }
        }
        out.flush()?;
        Ok(())
    }
}

/// Proyecta un punto de profundidad al frame RGB y devuelve su color.
/// Replica la transformación de `generatePoint` del SDK.
fn sample_color(
    x: f32,
    y: f32,
    z: f32,
    rgb: &RgbFrame,
    ri: &Intrinsics,
    ext: &Extrinsics,
) -> [u8; 3] {
    let r = &ext.rotation;
    let t = &ext.translation;
    let (tx, ty, tz) = (x + t[0], y + t[1], z + t[2]);
    let x2 = tx * r[0] + ty * r[1] + tz * r[2];
    let y2 = tx * r[3] + ty * r[4] + tz * r[5];
    let z2 = tx * r[6] + ty * r[7] + tz * r[8];
    if z2 == 0.0 {
        return [180, 180, 180];
    }
    let fu = ri.fx * x2 / z2 + ri.cx;
    let fv = ri.fy * y2 / z2 + ri.cy;
    if ri.width <= 0 || ri.height <= 0 {
        return [180, 180, 180];
    }
    // Normalizar por la resolución de calibración y mapear al frame RGB real.
    let u = (fu / ri.width as f32 * rgb.width as f32) as i32;
    let v = (fv / ri.height as f32 * rgb.height as f32) as i32;
    if u < 0 || v < 0 || u >= rgb.width as i32 || v >= rgb.height as i32 {
        return [180, 180, 180];
    }
    let idx = ((v * rgb.width as i32 + u) * 3) as usize;
    if idx + 2 < rgb.rgb.len() {
        [rgb.rgb[idx], rgb.rgb[idx + 1], rgb.rgb[idx + 2]]
    } else {
        [180, 180, 180]
    }
}

/// Cámara orbital para el visor 3D.
#[derive(Debug, Clone, Copy)]
pub struct OrbitCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        OrbitCamera {
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
        }
    }
}

/// Rasteriza la nube a un buffer RGB de tamaño (w,h) con z-buffer.
/// Devuelve los píxeles en orden RGB entrelazado.
pub fn render(
    cloud: &PointCloud,
    cam: OrbitCamera,
    w: usize,
    h: usize,
    tint: Option<[f32; 3]>,
) -> Vec<u8> {
    let mut pixels = vec![15u8; w * h * 3]; // fondo gris oscuro
    let mut zbuf = vec![f32::INFINITY; w * h];
    if cloud.points.is_empty() || w == 0 || h == 0 {
        return pixels;
    }

    // Centroide y extensión para encuadrar automáticamente.
    let mut c = [0.0f32; 3];
    for p in &cloud.points {
        c[0] += p.x;
        c[1] += p.y;
        c[2] += p.z;
    }
    let n = cloud.points.len() as f32;
    c = [c[0] / n, c[1] / n, c[2] / n];

    let mut extent = 1.0f32;
    for p in &cloud.points {
        extent = extent
            .max((p.x - c[0]).abs())
            .max((p.y - c[1]).abs())
            .max((p.z - c[2]).abs());
    }

    let (sy, cyaw) = cam.yaw.sin_cos();
    let (sp, cp) = cam.pitch.sin_cos();

    // Distancia de cámara y focal en píxeles.
    let dist = extent * 3.0 / cam.zoom.max(0.05);
    let focal = (w.min(h) as f32) * 0.9;

    for p in &cloud.points {
        // Centrar.
        let (px, py, pz) = (p.x - c[0], p.y - c[1], p.z - c[2]);
        // Rotar: yaw alrededor de Y, luego pitch alrededor de X.
        let rx = px * cyaw + pz * sy;
        let rz = -px * sy + pz * cyaw;
        let ry = py * cp - rz * sp;
        let rz2 = py * sp + rz * cp;
        // Trasladar a espacio cámara (cámara mirando +Z).
        let zc = rz2 + dist;
        if zc <= 1.0 {
            continue;
        }
        let su = (w as f32) * 0.5 + focal * rx / zc;
        // Y de pantalla hacia abajo: invertir.
        let sv = (h as f32) * 0.5 - focal * ry / zc;
        let iu = su as i32;
        let iv = sv as i32;
        if iu < 0 || iv < 0 || iu >= w as i32 || iv >= h as i32 {
            continue;
        }
        let idx = (iv as usize) * w + (iu as usize);
        if zc < zbuf[idx] {
            zbuf[idx] = zc;
            let o = idx * 3;
            let rgb = match tint {
                Some(t) => [
                    (p.rgb[0] as f32 * t[0]).min(255.0) as u8,
                    (p.rgb[1] as f32 * t[1]).min(255.0) as u8,
                    (p.rgb[2] as f32 * t[2]).min(255.0) as u8,
                ],
                None => p.rgb,
            };
            pixels[o] = rgb[0];
            pixels[o + 1] = rgb[1];
            pixels[o + 2] = rgb[2];
        }
    }

    pixels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::DepthFrame;

    fn intr(w: i16, h: i16, fx: f32, fy: f32, cx: f32, cy: f32) -> Intrinsics {
        Intrinsics {
            width: w,
            height: h,
            fx,
            cx,
            fy,
            cy,
            ..Default::default()
        }
    }

    #[test]
    fn deprojects_center_pixel_to_axis() {
        // Frame 3x3, intrínsecos a la misma resolución (escala 1).
        // Centro (u=1,v=1)=cx,cy → x=y=0; depth=100, scale=0.1 → z=10mm.
        let mut depth = vec![0u16; 9];
        depth[1 * 3 + 1] = 100;
        let frame = DepthFrame {
            width: 3,
            height: 3,
            depth,
            timestamp_ms: 0.0,
        };
        let params = CloudParams {
            depth_intr: intr(3, 3, 100.0, 100.0, 1.0, 1.0),
            rgb_intr: None,
            extrinsics: Extrinsics::default(),
            depth_scale: 0.1,
            clip_min_mm: 0.0,
            clip_max_mm: 0.0,
            roi: None,
        };
        let cloud = PointCloud::generate(&frame, None, &params);
        assert_eq!(cloud.points.len(), 1);
        let p = cloud.points[0];
        assert!((p.x).abs() < 1e-4, "x={}", p.x);
        assert!((p.y).abs() < 1e-4, "y={}", p.y);
        assert!((p.z - 10.0).abs() < 1e-4, "z={}", p.z);
        assert!(!cloud.has_color);
    }

    #[test]
    fn intrinsics_scale_to_frame_resolution() {
        // Calibración a 6x6 pero frame a 3x3 → escala 0.5.
        // Píxel (u=2,v=1), cx=cy=1 (en escala), fx=fy=50 (en escala).
        let mut depth = vec![0u16; 9];
        depth[1 * 3 + 2] = 50; // z = 5mm
        let frame = DepthFrame {
            width: 3,
            height: 3,
            depth,
            timestamp_ms: 0.0,
        };
        let params = CloudParams {
            depth_intr: intr(6, 6, 100.0, 100.0, 2.0, 2.0),
            rgb_intr: None,
            extrinsics: Extrinsics::default(),
            depth_scale: 0.1,
            clip_min_mm: 0.0,
            clip_max_mm: 0.0,
            roi: None,
        };
        // Escala 0.5: fx'=50, cx'=1. u=2 → x=(2-1)*5/50=0.1; v=1 → y=(1-1)*5/50=0.
        let cloud = PointCloud::generate(&frame, None, &params);
        assert_eq!(cloud.points.len(), 1);
        let p = cloud.points[0];
        assert!((p.x - 0.1).abs() < 1e-4, "x={}", p.x);
        assert!((p.y).abs() < 1e-4, "y={}", p.y);
        assert!((p.z - 5.0).abs() < 1e-4, "z={}", p.z);
    }

    #[test]
    fn skips_zero_depth() {
        let frame = DepthFrame {
            width: 2,
            height: 2,
            depth: vec![0, 0, 0, 0],
            timestamp_ms: 0.0,
        };
        let params = CloudParams {
            depth_intr: intr(2, 2, 10.0, 10.0, 1.0, 1.0),
            rgb_intr: None,
            extrinsics: Extrinsics::default(),
            depth_scale: 0.1,
            clip_min_mm: 0.0,
            clip_max_mm: 0.0,
            roi: None,
        };
        let cloud = PointCloud::generate(&frame, None, &params);
        assert!(cloud.points.is_empty());
    }

    #[test]
    fn ply_export_has_valid_header() {
        let cloud = PointCloud {
            points: vec![
                Point { x: 1.0, y: 2.0, z: 3.0, rgb: [10, 20, 30] },
                Point { x: 4.0, y: 5.0, z: 6.0, rgb: [40, 50, 60] },
            ],
            has_color: true,
        };
        let dir = std::env::temp_dir();
        let path = dir.join("revoscan_test_cloud.ply");
        cloud.export_ply(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert!(text.starts_with("ply\n"));
        assert!(text.contains("element vertex 2"));
        assert!(text.contains("property uchar red"));
        assert!(text.contains("1 2 3 10 20 30"));
    }
}
