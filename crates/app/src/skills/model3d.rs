//! CPU preview of 3D models (components/Assets/ModelViewer.tsx without three.js): loads glTF/GLB (and the
//! GLB export of a .blend), OBJ (+ MTL colors) and STL, then rasterizes them with a z-buffer into a BGRA
//! image GPUI can draw. A second buffer keeps the part under each pixel, so a click picks a part.
//! shortcut: flat colors (a texture counts as its average color), no FBX; upgrade to a wgpu view if
//! textured previews matter.

use std::path::Path;

/// Models without materials (OBJ without .mtl, STL) get a warm clay color instead of flat white.
const CLAY: [f32; 3] = [0.784, 0.635, 0.478];
const SELECTED: [f32; 3] = [0.290, 0.871, 0.502];
const FOV_Y: f32 = std::f32::consts::FRAC_PI_4;

type Vec3 = [f32; 3];

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn normalize(a: Vec3) -> Vec3 {
    let len = dot(a, a).sqrt();
    if len > 0.0 { [a[0] / len, a[1] / len, a[2] / len] } else { [0.0, 1.0, 0.0] }
}

/// One named object of the model, triangles in model space.
pub struct Part {
    pub name: String,
    color: Vec3,
    tris: Vec<[Vec3; 3]>,
}

pub struct Model {
    pub parts: Vec<Part>,
    center: Vec3,
    /// Bounding box diagonal (the original frames the model with it).
    size: f32,
}

/// Orbit camera around the model center.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    pub yaw: f32,
    pub pitch: f32,
    /// Camera distance in model sizes.
    pub distance: f32,
}

impl Default for Orbit {
    /// The original's 3/4 view: camera at (0.7, 0.5, 0.9) x size.
    fn default() -> Self {
        let (x, y, z) = (0.7f32, 0.5f32, 0.9f32);
        let distance = (x * x + y * y + z * z).sqrt();
        Self { yaw: x.atan2(z), pitch: (y / distance).asin(), distance }
    }
}

impl Orbit {
    pub fn rotate(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.01;
        self.pitch = (self.pitch + dy * 0.01).clamp(-1.5, 1.5);
    }

    pub fn zoom(&mut self, steps: f32) {
        self.distance = (self.distance * 1.1f32.powf(steps)).clamp(0.3, 20.0);
    }
}

/// A rendered frame: BGRA pixels (GPUI's image layout) and the part index + 1 under each pixel.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub ids: Vec<u32>,
}

impl Frame {
    /// Index of the part at a pixel, if any.
    pub fn pick(&self, model: &Model, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let id = self.ids[(y * self.width + x) as usize];
        (id > 0).then(|| id as usize - 1).filter(|i| *i < model.parts.len())
    }
}

/// Generated models name parts `Wall__6b4a32`: that hex is their color.
fn color_from_name(name: &str) -> Option<Vec3> {
    let (_, hex) = name.rsplit_once("__")?;
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some([((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0])
}

/// sRGB to linear, so lighting happens in linear space like three.js.
fn linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

pub fn load(path: &Path, ext: &str) -> Result<Model, String> {
    // The gltf crate panics on some malformed files (out-of-range buffer views): never take the app down.
    std::panic::catch_unwind(|| load_model(path, ext))
        .unwrap_or_else(|_| Err("Could not read the model (malformed file).".into()))
}

fn load_model(path: &Path, ext: &str) -> Result<Model, String> {
    let parts = match ext {
        "glb" | "gltf" | "blend" => load_gltf(path)?,
        "obj" => load_obj(path)?,
        "stl" => load_stl(path)?,
        "fbx" => return Err("FBX models cannot be previewed here. Describe the change below.".into()),
        other => return Err(format!("Unsupported format: .{other}")),
    };
    Model::new(parts)
}

impl Model {
    fn new(parts: Vec<Part>) -> Result<Self, String> {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for v in parts.iter().flat_map(|p| p.tris.iter().flatten()) {
            for i in 0..3 {
                min[i] = min[i].min(v[i]);
                max[i] = max[i].max(v[i]);
            }
        }
        if !min[0].is_finite() {
            return Err("This model has no visible geometry.".into());
        }
        let center = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0, (min[2] + max[2]) / 2.0];
        let size = dot(sub(max, min), sub(max, min)).sqrt();
        Ok(Self { parts, center, size: if size > 0.0 { size } else { 1.0 } })
    }

    pub fn part_name(&self, index: usize) -> String {
        self.parts
            .get(index)
            .map(|p| p.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "unnamed part".into())
    }
}

type Mat4 = [[f32; 4]; 4];

fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    // Column-major, like glTF: m[col][row].
    let mut out = [[0.0; 4]; 4];
    for (c, col) in out.iter_mut().enumerate() {
        for (r, cell) in col.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

fn transform(m: &Mat4, v: Vec3) -> Vec3 {
    let w = m[0][3] * v[0] + m[1][3] * v[1] + m[2][3] * v[2] + m[3][3];
    let w = if w.abs() > f32::EPSILON { w } else { 1.0 };
    [
        (m[0][0] * v[0] + m[1][0] * v[1] + m[2][0] * v[2] + m[3][0]) / w,
        (m[0][1] * v[0] + m[1][1] * v[1] + m[2][1] * v[2] + m[3][1]) / w,
        (m[0][2] * v[0] + m[1][2] * v[1] + m[2][2] * v[2] + m[3][2]) / w,
    ]
}

/// Average color of a texture, so textured parts keep their overall tint.
fn average_color(image: &gltf::image::Data) -> Option<Vec3> {
    use gltf::image::Format;
    let stride = match image.format {
        Format::R8G8B8 => 3,
        Format::R8G8B8A8 => 4,
        _ => return None,
    };
    let mut sum = [0.0f64; 3];
    let mut count = 0.0f64;
    for px in image.pixels.chunks_exact(stride).step_by(7) {
        for i in 0..3 {
            sum[i] += linear(px[i] as f32 / 255.0) as f64;
        }
        count += 1.0;
    }
    (count > 0.0).then(|| [(sum[0] / count) as f32, (sum[1] / count) as f32, (sum[2] / count) as f32])
}

fn load_gltf(path: &Path) -> Result<Vec<Part>, String> {
    let (doc, buffers, images) = gltf::import(path).map_err(|e| format!("Could not read the model: {e}"))?;
    let scene = doc.default_scene().or_else(|| doc.scenes().next()).ok_or("The model has no scene.")?;
    let mut parts = Vec::new();
    let identity: Mat4 = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let mut stack: Vec<(gltf::Node, Mat4, String)> = scene.nodes().map(|n| (n, identity, String::new())).collect();
    while let Some((node, parent, parent_name)) = stack.pop() {
        let world = mul(&parent, &node.transform().matrix());
        // Nearest named ancestor: Blender exports often leave the mesh nodes unnamed.
        let name = node.name().filter(|n| !n.is_empty()).map(str::to_owned).unwrap_or(parent_name);
        if let Some(mesh) = node.mesh() {
            let mesh_name = if name.is_empty() { mesh.name().unwrap_or("").to_owned() } else { name.clone() };
            let mut tris = Vec::new();
            let mut color = None;
            for prim in mesh.primitives().filter(|p| p.mode() == gltf::mesh::Mode::Triangles) {
                let reader = prim.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
                let Some(positions) = reader.read_positions() else { continue };
                let positions: Vec<Vec3> = positions.map(|p| transform(&world, p)).collect();
                let indices: Vec<u32> = match reader.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };
                for t in indices.chunks_exact(3) {
                    if let (Some(a), Some(b), Some(c)) =
                        (positions.get(t[0] as usize), positions.get(t[1] as usize), positions.get(t[2] as usize))
                    {
                        tris.push([*a, *b, *c]);
                    }
                }
                if color.is_none() {
                    let pbr = prim.material().pbr_metallic_roughness();
                    let f = pbr.base_color_factor();
                    let texture = pbr
                        .base_color_texture()
                        .and_then(|t| images.get(t.texture().source().index()))
                        .and_then(average_color);
                    color = Some(match texture {
                        Some(t) => [t[0] * f[0], t[1] * f[1], t[2] * f[2]],
                        // glTF factors are linear already; name colors only for untextured parts.
                        None => color_from_name(&mesh_name).map(|c| c.map(linear)).unwrap_or([f[0], f[1], f[2]]),
                    });
                }
            }
            if !tris.is_empty() {
                parts.push(Part { name: mesh_name, color: color.unwrap_or(CLAY.map(linear)), tris });
            }
        }
        for child in node.children() {
            stack.push((child, world, name.clone()));
        }
    }
    Ok(parts)
}

fn load_obj(path: &Path) -> Result<Vec<Part>, String> {
    let options = tobj::LoadOptions { triangulate: true, single_index: true, ..Default::default() };
    let (models, materials) = tobj::load_obj(path, &options).map_err(|e| format!("Could not read the model: {e}"))?;
    let materials = materials.unwrap_or_default();
    Ok(models
        .into_iter()
        .map(|m| {
            let mesh = m.mesh;
            let vertex = |i: u32| {
                let i = i as usize * 3;
                [
                    mesh.positions.get(i).copied().unwrap_or(0.0),
                    mesh.positions.get(i + 1).copied().unwrap_or(0.0),
                    mesh.positions.get(i + 2).copied().unwrap_or(0.0),
                ]
            };
            let tris = mesh.indices.chunks_exact(3).map(|t| [vertex(t[0]), vertex(t[1]), vertex(t[2])]).collect();
            let diffuse = mesh.material_id.and_then(|i| materials.get(i)).and_then(|mat| mat.diffuse);
            let color = diffuse
                .map(|d| d.map(linear))
                .or_else(|| color_from_name(&m.name).map(|c| c.map(linear)))
                .unwrap_or(CLAY.map(linear));
            Part { name: m.name, color, tris }
        })
        .collect())
}

fn load_stl(path: &Path) -> Result<Vec<Part>, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("Could not open the model: {e}"))?;
    let mesh = stl_io::read_stl(&mut file).map_err(|e| format!("Could not read the model: {e}"))?;
    let vertex = |i: usize| mesh.vertices.get(i).map(|v| [v[0], v[1], v[2]]).unwrap_or([0.0; 3]);
    let tris = mesh
        .faces
        .iter()
        .map(|f| [vertex(f.vertices[0]), vertex(f.vertices[1]), vertex(f.vertices[2])])
        .collect();
    Ok(vec![Part { name: String::new(), color: CLAY.map(linear), tris }])
}

/// Hemisphere sky light + a sun from (3, 5, 4) + a soft ambient (the original's room environment).
fn shade(color: Vec3, normal: Vec3, view_dir: Vec3) -> Vec3 {
    // Two-sided: flip normals facing away from the camera (models with mixed winding).
    let n = if dot(normal, view_dir) < 0.0 { normal.map(|c| -c) } else { normal };
    let sun = normalize([3.0, 5.0, 4.0]);
    let diffuse = dot(n, sun).max(0.0) * 1.3;
    let up = (n[1] + 1.0) / 2.0;
    let sky = [1.0, 1.0, 1.0];
    let ground = [0.267, 0.267, 0.4];
    let mut out = [0.0; 3];
    for i in 0..3 {
        let hemi = (ground[i] + (sky[i] - ground[i]) * up) * 0.6;
        let lit = color[i] * (diffuse + hemi + 0.35);
        // Soft tone curve so bright parts do not clip.
        out[i] = lit / (1.0 + lit * 0.35);
    }
    out
}

/// Renders the model at `width` x `height` pixels on a transparent background.
pub fn render(model: &Model, orbit: &Orbit, width: u32, height: u32, selected: Option<usize>) -> Frame {
    let (w, h) = (width.max(1), height.max(1));
    let mut bgra = vec![0u8; (w * h * 4) as usize];
    let mut ids = vec![0u32; (w * h) as usize];
    let mut depth = vec![f32::INFINITY; (w * h) as usize];

    let dist = orbit.distance * model.size;
    let (sy, cy) = orbit.yaw.sin_cos();
    let (sp, cp) = orbit.pitch.sin_cos();
    let eye_dir = [sy * cp, sp, cy * cp];
    let eye = [model.center[0] + eye_dir[0] * dist, model.center[1] + eye_dir[1] * dist, model.center[2] + eye_dir[2] * dist];
    let forward = normalize(sub(model.center, eye));
    let right = normalize(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    let near = model.size / 100.0;
    let focal = (h as f32 / 2.0) / (FOV_Y / 2.0).tan();
    let (half_w, half_h) = (w as f32 / 2.0, h as f32 / 2.0);

    for (index, part) in model.parts.iter().enumerate() {
        let base = if selected == Some(index) {
            let s = SELECTED.map(linear);
            [(part.color[0] + s[0]) / 2.0, (part.color[1] + s[1]) / 2.0, (part.color[2] + s[2]) / 2.0]
        } else {
            part.color
        };
        for tri in &part.tris {
            let normal = normalize(cross(sub(tri[1], tri[0]), sub(tri[2], tri[0])));
            let to_eye = normalize(sub(eye, tri[0]));
            let rgb = shade(base, normal, to_eye);
            let px = [to_srgb(rgb[2]), to_srgb(rgb[1]), to_srgb(rgb[0]), 255];
            // Camera space: x right, y up, z forward (depth).
            let cam = tri.map(|v| {
                let d = sub(v, eye);
                [dot(d, right), dot(d, up), dot(d, forward)]
            });
            // shortcut: triangles crossing the near plane are skipped, not clipped (zoom keeps the camera outside).
            if cam.iter().any(|c| c[2] < near) {
                continue;
            }
            let screen = cam.map(|c| [half_w + c[0] * focal / c[2], half_h - c[1] * focal / c[2], 1.0 / c[2]]);
            raster(&screen, w, h, |i, inv_z| {
                let z = 1.0 / inv_z;
                if z < depth[i] {
                    depth[i] = z;
                    ids[i] = index as u32 + 1;
                    bgra[i * 4..i * 4 + 4].copy_from_slice(&px);
                }
            });
        }
    }
    Frame { width: w, height: h, bgra, ids }
}

/// Fills a screen-space triangle (x, y, 1/z), calling `plot(pixel index, 1/z)` per covered pixel center.
fn raster(t: &[[f32; 3]; 3], w: u32, h: u32, mut plot: impl FnMut(usize, f32)) {
    let area = (t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1]);
    if area.abs() < 1e-6 {
        return;
    }
    let min_x = t.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min).floor().max(0.0) as u32;
    let max_x = t.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max).ceil().min(w as f32 - 1.0);
    let min_y = t.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min).floor().max(0.0) as u32;
    let max_y = t.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max).ceil().min(h as f32 - 1.0);
    if max_x < 0.0 || max_y < 0.0 {
        return;
    }
    let edge = |a: &[f32; 3], b: &[f32; 3], x: f32, y: f32| (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
    for y in min_y..=max_y as u32 {
        for x in min_x..=max_x as u32 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = edge(&t[1], &t[2], px, py) / area;
            let w1 = edge(&t[2], &t[0], px, py) / area;
            let w2 = edge(&t[0], &t[1], px, py) / area;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            plot((y * w + x) as usize, w0 * t[0][2] + w1 * t[1][2] + w2 * t[2][2]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    fn cube() -> Model {
        let v = |x: f32, y: f32, z: f32| [x, y, z];
        let quads = [
            [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)],
            [v(1., 0., 0.), v(0., 0., 0.), v(0., 1., 0.), v(1., 1., 0.)],
            [v(0., 1., 0.), v(0., 1., 1.), v(1., 1., 1.), v(1., 1., 0.)],
        ];
        let tris = quads.iter().flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]]).collect();
        Model::new(vec![Part { name: "Box__ff0000".into(), color: [1.0, 0.0, 0.0], tris }]).unwrap()
    }

    #[test]
    fn renders_and_picks_the_center() {
        let model = cube();
        let frame = render(&model, &Orbit::default(), 64, 48, None);
        assert_eq!(frame.bgra.len(), 64 * 48 * 4);
        assert_eq!(frame.pick(&model, 32, 24), Some(0));
        assert_eq!(frame.pick(&model, 0, 0), None);
        assert_eq!(model.part_name(0), "Box__ff0000");
        // Background stays transparent.
        assert_eq!(frame.bgra[3], 0);
    }

    #[test]
    fn name_colors_and_orbit_limits() {
        assert_eq!(color_from_name("Wall__ff0000"), Some([1.0, 0.0, 0.0]));
        assert_eq!(color_from_name("Wall"), None);
        let mut orbit = Orbit::default();
        orbit.rotate(0.0, 10_000.0);
        assert!(orbit.pitch <= 1.5);
        orbit.zoom(-1000.0);
        assert!(orbit.distance >= 0.3);
        assert!(Model::new(Vec::new()).is_err());
    }
}
