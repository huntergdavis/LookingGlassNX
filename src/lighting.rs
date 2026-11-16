//! Bounded original-declaration lights, occluded flares and projected contact shadows.
use crate::{assets::Assets, collision::World, texture};
use anyhow::Result;
use macroquad::prelude::*;
use std::cell::RefCell;

#[derive(Clone, Copy, Debug)]
pub struct Light {
    pub position: Vec3,
    pub color: Vec3,
    pub radius: f32,
    pub only_models: bool,
    pub flare: bool,
}
thread_local! {
    static LIGHTS: RefCell<Vec<Light>> = const {RefCell::new(Vec::new())};
    static SHADOW: RefCell<Option<Texture2D>> = const {RefCell::new(None)};
    static FLARE: RefCell<Option<Texture2D>> = const {RefCell::new(None)};
    static CROQUET: RefCell<Vec<Light>> = const {RefCell::new(Vec::new())};
}
// Horizon only: the shadow and flare textures live in process-wide Mutex slots instead of
// thread_local RefCells. Logs 021-023 end at the first SHADOW access in load_art, so the
// thread_local access is the suspected fault. Both slots are read and written on the one
// game thread, so the values seen by load_art, shadow() and flares() are the same as before.
#[cfg(target_os = "horizon")]
static SHADOW_HORIZON: std::sync::Mutex<Option<Texture2D>> = std::sync::Mutex::new(None);
#[cfg(target_os = "horizon")]
static FLARE_HORIZON: std::sync::Mutex<Option<Texture2D>> = std::sync::Mutex::new(None);
fn has_shadow() -> bool {
    #[cfg(target_os = "horizon")]
    {
        SHADOW_HORIZON.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }
    #[cfg(not(target_os = "horizon"))]
    {
        SHADOW.with(|s| s.borrow().is_some())
    }
}
fn shadow_texture() -> Option<Texture2D> {
    #[cfg(target_os = "horizon")]
    {
        SHADOW_HORIZON.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    #[cfg(not(target_os = "horizon"))]
    {
        SHADOW.with(|s| s.borrow().clone())
    }
}
fn flare_texture() -> Option<Texture2D> {
    #[cfg(target_os = "horizon")]
    {
        FLARE_HORIZON.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    #[cfg(not(target_os = "horizon"))]
    {
        FLARE.with(|s| s.borrow().clone())
    }
}
fn set_shadow(texture: Texture2D) {
    #[cfg(target_os = "horizon")]
    {
        *SHADOW_HORIZON.lock().unwrap_or_else(|e| e.into_inner()) = Some(texture);
    }
    #[cfg(not(target_os = "horizon"))]
    {
        SHADOW.with(|s| *s.borrow_mut() = Some(texture));
    }
}
fn set_flare(texture: Texture2D) {
    #[cfg(target_os = "horizon")]
    {
        *FLARE_HORIZON.lock().unwrap_or_else(|e| e.into_inner()) = Some(texture);
    }
    #[cfg(not(target_os = "horizon"))]
    {
        FLARE.with(|s| *s.borrow_mut() = Some(texture));
    }
}
pub fn select(mut lights: Vec<Light>, eye: Vec3, world: &World) {
    lights.retain(|l| {
        l.position.is_finite() && l.color.is_finite() && l.radius > 0. && l.radius <= 8192.
    });
    lights.sort_by(|a, b| score(*b, eye).total_cmp(&score(*a, eye)));
    lights.truncate(32);
    // A bounded visibility query rejects lights fully hidden from this room.
    lights.retain(|l| world.sweep(eye, l.position, Vec3::ZERO).fraction >= 0.995);
    lights.truncate(8);
    LIGHTS.with(|v| *v.borrow_mut() = lights);
}
fn score(light: Light, eye: Vec3) -> f32 {
    light.radius / (light.position.distance(eye) + light.radius)
}
pub(crate) fn fixture_lights(lights: Vec<Light>) {
    LIGHTS.with(|v| *v.borrow_mut() = lights.into_iter().take(8).collect());
}
pub(crate) fn count() -> usize {
    LIGHTS.with(|l| l.borrow().len())
}
pub fn uniforms() -> Vec<UniformDesc> {
    (0..8)
        .flat_map(|i| {
            [
                UniformDesc::new(&format!("FxLight{i}"), UniformType::Float4),
                UniformDesc::new(&format!("FxColor{i}"), UniformType::Float4),
            ]
        })
        .collect()
}
pub fn fragment(source: &str) -> String {
    let mut s = String::new();
    for i in 0..8 {
        s += &format!("uniform vec4 FxLight{i}; uniform vec4 FxColor{i};\n");
    }
    s += "vec3 dynamicLight(vec3 position){vec3 light=vec3(0.0);\n";
    for i in 0..8 {
        s+=&format!("if(FxColor{i}.w>0.5)light+=FxColor{i}.rgb*max(0.0,1.0-distance(position,FxLight{i}.xyz)/max(FxLight{i}.w,0.001));\n");
    }
    s += "return light;}\n";
    source.replace("// LIGHTS", &s)
}
pub fn apply(material: &Material, model: bool, enabled: bool) {
    LIGHTS.with(|lights| {
        let lights = lights.borrow();
        for i in 0..8 {
            let light = lights
                .get(i)
                .filter(|l| enabled && (model || !l.only_models));
            material.set_uniform(
                &format!("FxLight{i}"),
                light.map_or(Vec4::ZERO, |l| l.position.extend(l.radius)),
            );
            material.set_uniform(
                &format!("FxColor{i}"),
                light.map_or(Vec4::ZERO, |l| l.color.extend(1.)),
            );
        }
    });
}
macro_rules! horizon_light_diag {
    ($($arg:tt)*) => {{
        #[cfg(target_os = "horizon")]
        crate::horizon_diag(&format!($($arg)*));
    }}
}

pub fn load_art(assets: &mut Assets) -> Result<()> {
    horizon_light_diag!("rust:lighting:load-art-enter");
    horizon_light_diag!("rust:lighting:shadow-check-begin");
    let shadow_loaded = has_shadow();
    horizon_light_diag!("rust:lighting:shadow-check-ready:loaded={shadow_loaded}");
    if shadow_loaded {
        horizon_light_diag!("rust:lighting:load-art-already-loaded");
        return Ok(());
    }
    horizon_light_diag!("rust:lighting:load-art-begin");
    horizon_light_diag!("rust:lighting:read-materials-begin");
    let specs = texture::read_materials(assets)?;
    horizon_light_diag!("rust:lighting:read-materials-ready:count={}", specs.len());
    horizon_light_diag!("rust:lighting:declared-lights-begin");
    let lights = crate::particles::declared_lights(assets, "croquetball");
    horizon_light_diag!(
        "rust:lighting:declared-lights-ready:ok={}:count={}",
        lights.is_ok(),
        lights.as_ref().map_or(0, |l| l.len())
    );
    CROQUET.with(|s| *s.borrow_mut() = lights.unwrap_or_default());
    for (slot, names) in [
        (false, &["markshadow", "textures/common/shadow"] as &[&str]),
        (true, &["flare", "lensflare", "textures/sprites/flare"]),
    ] {
        for name in names {
            horizon_light_diag!("rust:lighting:resolve-begin:{name}");
            let resolved = texture::resolve(assets, name, &specs);
            horizon_light_diag!("rust:lighting:resolve-ready:{name}:found={}", resolved.is_some());
            if let Some(path) = resolved {
                horizon_light_diag!("rust:lighting:decode-begin:{name}");
                let image = match texture::decode(assets, &path) {
                    Ok(image) => image,
                    Err(e) => {
                        horizon_light_diag!("rust:lighting:decode-failed:{name}");
                        return Err(e);
                    }
                };
                horizon_light_diag!(
                    "rust:lighting:decode-ready:{name}:width={}:height={}:bytes={}",
                    image.width,
                    image.height,
                    image.pixels.len()
                );
                let t = Texture2D::from_rgba8(image.width, image.height, &image.pixels);
                horizon_light_diag!("rust:lighting:upload-ready:{name}");
                t.set_filter(FilterMode::Linear);
                if slot {
                    set_flare(t);
                } else {
                    set_shadow(t);
                }
                break;
            }
        }
    }
    horizon_light_diag!("rust:lighting:load-art-ready");
    Ok(())
}
pub fn croquet(position: Vec3) -> Vec<Light> {
    CROQUET.with(|s| {
        s.borrow()
            .iter()
            .map(|l| Light {
                position: position + l.position,
                ..*l
            })
            .collect()
    })
}
pub fn shadow(world: &World, feet: Vec3, radius: f32) {
    let Some(texture) = shadow_texture() else {
        return;
    };
    let hit = world.sweep(feet + Vec3::Z * 4., feet - Vec3::Z * 160., Vec3::ZERO);
    if hit.fraction >= 1. || hit.normal.z < 0.4 {
        return;
    }
    let center = (feet + Vec3::Z * 4.).lerp(feet - Vec3::Z * 160., hit.fraction) + hit.normal * 0.3;
    let strength = (0.55 * (1. - feet.distance(center) / 160.)).clamp(0., 0.55);
    let tangent = hit.normal.cross(Vec3::X).normalize_or_zero();
    let bitangent = hit.normal.cross(tangent);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for (position, uv) in [(center, Vec2::splat(0.5))]
        .into_iter()
        .chain((0..=24).map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 24.;
            (
                center + (tangent * a.cos() + bitangent * a.sin()) * radius,
                vec2(a.cos(), a.sin()) * 0.5 + Vec2::splat(0.5),
            )
        }))
    {
        // Clip each rim point against nearby support; shadows never bridge a pit.
        let h = world.sweep(
            position + hit.normal * 2.,
            position - hit.normal * 5.,
            Vec3::ZERO,
        );
        vertices.push(Vertex {
            position,
            uv,
            normal: Vec4::ZERO,
            color: [
                0,
                0,
                0,
                if h.fraction < 1. {
                    (strength * 255.) as u8
                } else {
                    0
                },
            ],
        });
    }
    for i in 1..=24 {
        indices.extend([0, i, i + 1]);
    }
    crate::render_fx::effect(
        &Mesh {
            vertices,
            indices,
            texture: Some(texture),
        },
        crate::materials::Blend::Alpha,
    );
}
pub fn flares(eye: Vec3, direction: Vec3) {
    let Some(texture) = flare_texture() else {
        return;
    };
    LIGHTS.with(|lights| {
        for l in lights.borrow().iter().filter(|l| l.flare) {
            let right = direction.cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X);
            let up = right.cross(direction).normalize_or_zero();
            let r = l.radius.clamp(8., 48.);
            let color: [u8; 4] = Color::new(
                l.color.x,
                l.color.y,
                l.color.z,
                (eye.distance(l.position) / 32.).min(1.),
            )
            .into();
            let vertices = [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)]
                .into_iter()
                .map(|(x, y)| Vertex {
                    position: l.position + (right * x + up * y) * r,
                    uv: vec2(x * 0.5 + 0.5, 0.5 - y * 0.5),
                    normal: Vec4::ZERO,
                    color,
                })
                .collect();
            crate::render_fx::effect(
                &Mesh {
                    vertices,
                    indices: vec![0, 1, 2, 0, 2, 3],
                    texture: Some(texture.clone()),
                },
                crate::materials::Blend::Add,
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn light_selection_is_bounded_and_rejects_nonfinite() {
        let mut lights = (0..100)
            .map(|i| Light {
                position: Vec3::X * i as f32,
                color: Vec3::ONE,
                radius: 100.,
                only_models: false,
                flare: false,
            })
            .collect::<Vec<_>>();
        lights.push(Light {
            position: Vec3::splat(f32::NAN),
            ..lights[0]
        });
        select(lights, Vec3::ZERO, &World::fixture(&[]));
        LIGHTS.with(|l| {
            let l = l.borrow();
            assert_eq!(l.len(), 8);
            assert_eq!(l[0].position, Vec3::ZERO);
        });
    }
    #[test]
    fn opaque_wall_rejects_light_and_flare() {
        let world = World::fixture(&[(vec3(5., -20., -20.), vec3(6., 20., 20.))]);
        select(
            vec![Light {
                position: Vec3::X * 10.,
                color: Vec3::ONE,
                radius: 100.,
                only_models: false,
                flare: true,
            }],
            Vec3::ZERO,
            &world,
        );
        assert_eq!(count(), 0);
    }
}
