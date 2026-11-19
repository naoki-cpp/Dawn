//! Bounded, direction-only galactic background generation.
//!
//! Ports the normalized exponential disks, Gaussian bar, blackbody proxy and
//! correlated lognormal extinction from `galactic_model.gd`/`galactic_dust.gdshaderinc`.
//! This is a synthetic observational population, not an astronomical map or
//! named-star catalog. The single 5000 K spectrum is a population approximation;
//! packet weights are not an individual-star luminosity function.
//! All transport and sampling calculations use f64; only output RGB is narrowed.
//!
//! Resolved stars are Monte Carlo packets of the *same attenuated*
//! emissivity as the panorama, not a second independent luminosity population.
//! Their expected RGB integral equals the light removed from the panorama.
//! Sampling uses pixel-center quadrature and uniform solid angle within a pixel;
//! finer resolution improves angular structure, not the physical source model.
//! Independent apparent-magnitude packet weights spanning ten magnitudes provide
//! a synthetic faint-dominated hierarchy. They are normalized to the exact
//! resolved scalar flux, not asserted to be measured stellar luminosities.
//! No generated position/distance should be used for camera parallax.

use std::f64::consts::{PI, TAU};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_PIXELS: u64 = 1_048_576;
const MAX_RAY_SAMPLES: u64 = 33_554_432;
const MAX_STARS: u32 = 100_000;
const SUN: [f64; 3] = [-8.2, 0.023, 0.0];
const WEIGHTS: [f64; 3] = [0.55, 0.30, 0.15];
const OFFSETS: [[f64; 3]; 3] = [[17.3, 5.7, 31.1], [43.1, 29.7, 11.9], [7.9, 53.3, 23.5]];
const EXTINCTION: [f64; 3] = [0.84, 1.0, 1.32];
const EMISSIVITY: f64 = 0.071 * 1000.0 * 0.00372 / (4.0 * PI);

/// Common generation settings. Every u64 seed, including zero, is supported.
///
/// Dimensions are independently 1..=2048, with at most 1,048,576 pixels and
/// 33,554,432 pixel/step evaluations. Steps are 4..=256; stars 0..=100,000.
/// Observer coordinates are galactic kpc, each within +/-30; path length is
/// 0.1..=50 kpc. `resolved_fraction` is 0..=1. With zero stars it is effectively
/// zero, so requesting no sprites never silently discards their light.
#[derive(Debug, Clone, PartialEq)]
pub struct GalaxyConfig {
    pub seed: u64,
    pub width: u32,
    pub height: u32,
    pub star_count: u32,
    pub integration_steps: u32,
    pub observer_kpc: [f64; 3],
    pub maximum_distance_kpc: f64,
    pub resolved_fraction: f64,
}

impl Default for GalaxyConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            width: 512,
            height: 256,
            star_count: 2000,
            integration_steps: 64,
            observer_kpc: SUN,
            maximum_distance_kpc: 22.0,
            resolved_fraction: 0.08,
        }
    }
}

/// One fixed sky direction carrying integrated scene-linear RGB flux.
/// `color` has mean channel value one; multiply it by `flux` to recover RGB.
/// Flux is radiance integrated over solid angle (not a per-pixel peak value).
#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundStar {
    pub direction: [f64; 3],
    pub color: [f32; 3],
    pub flux: f64,
}

/// Row-major linear HDR RGB radiance, north pole at the top, and sky sprites.
///
/// Pixel centers use `u=(x+0.5)/width`, `v=(y+0.5)/height` and direction
/// `[-sin(TAU*u)*sin(PI*v), cos(PI*v), -cos(TAU*u)*sin(PI*v)]`.
/// Inverse U is `atan2(-dir.x,-dir.z)/TAU`, wrapped to [0,1).
#[derive(Debug, Clone, PartialEq)]
pub struct GalaxyBackground {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 3]>,
    pub stars: Vec<BackgroundStar>,
}

/// Validation and generation failures; cancellation returns no partial image.
#[derive(Debug, Clone, PartialEq)]
pub enum GalaxyError {
    InvalidSetting {
        field: &'static str,
        value: f64,
        minimum: f64,
        maximum: f64,
    },
    BudgetExceeded {
        budget: &'static str,
        requested: u64,
        maximum: u64,
    },
    AllocationFailed {
        buffer: &'static str,
    },
    Cancelled,
}

impl std::fmt::Display for GalaxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSetting {
                field,
                value,
                minimum,
                maximum,
            } => write!(
                f,
                "{field}={value} must be finite and within {minimum}..={maximum}"
            ),
            Self::BudgetExceeded {
                budget,
                requested,
                maximum,
            } => write!(f, "{budget} budget {requested} exceeds {maximum}"),
            Self::AllocationFailed { buffer } => write!(f, "cannot allocate galaxy {buffer}"),
            Self::Cancelled => f.write_str("galaxy generation cancelled"),
        }
    }
}

impl std::error::Error for GalaxyError {}

/// Generate on the caller's thread. Adapters own asynchronous scheduling and
/// must discard stale results when changing systems. Cancellation is checked
/// before allocation, each ray, every eight ray steps, and every star.
///
/// ```
/// use dawn_client_core::{generate_background, GalaxyConfig};
/// use std::sync::atomic::AtomicBool;
/// let config = GalaxyConfig { width: 16, height: 8, ..GalaxyConfig::default() };
/// let sky = generate_background(&config, &AtomicBool::new(false))?;
/// assert_eq!(sky.pixels.len(), 128);
/// # Ok::<(), dawn_client_core::GalaxyError>(())
/// ```
pub fn generate_background(
    config: &GalaxyConfig,
    cancelled: &AtomicBool,
) -> Result<GalaxyBackground, GalaxyError> {
    validate(config)?;
    check_cancelled(cancelled)?;
    let count = config.width as usize * config.height as usize;
    let mut radiance = buffer(count, "radiance")?;
    let mut cumulative = buffer(count, "sampling distribution")?;
    let model = GalaxyModel::new(config);
    let mut total = 0.0;
    for y in 0..config.height {
        let area = pixel_area(config, y);
        for x in 0..config.width {
            check_cancelled(cancelled)?;
            let direction = pixel_direction(config, x, y);
            let rgb = model.integrate(to_galactic(direction), cancelled)?;
            total += mean(rgb) * area;
            radiance.push(rgb);
            cumulative.push(total);
        }
    }
    let fraction = if config.star_count == 0 {
        0.0
    } else {
        config.resolved_fraction
    };
    let mut stars = buffer(config.star_count as usize, "stars")?;
    let mut rng = Random(config.seed ^ 0xa076_1d64_78bd_642f);
    let mut weight_rng = Random(config.seed ^ 0xe703_7ed1_a0b4_28db);
    let mut weight_sum = 0.0;
    if total > 0.0 && fraction > 0.0 {
        for _ in 0..config.star_count {
            check_cancelled(cancelled)?;
            let selection = rng.next() * total;
            let index = cumulative.partition_point(|v| *v <= selection);
            // Multiplication can round an upper-end variate to total. Choose
            // the last nonzero bin rather than a potentially empty trailing bin.
            let index = if index == count {
                cumulative.partition_point(|v| *v < total)
            } else {
                index
            };
            let x = index as u32 % config.width;
            let y = index as u32 / config.width;
            let longitude = TAU * (f64::from(x) + rng.next()) / f64::from(config.width);
            let north = (PI * f64::from(y) / f64::from(config.height)).cos();
            let south = (PI * f64::from(y + 1) / f64::from(config.height)).cos();
            let cosine = north + (south - north) * rng.next();
            let rgb = radiance[index];
            let intensity = mean(rgb);
            let weight = apparent_flux_weight(weight_rng.next());
            weight_sum += weight;
            stars.push(BackgroundStar {
                direction: direction(longitude, cosine),
                color: rgb.map(|v| (v / intensity) as f32),
                flux: weight,
            });
        }
        let resolved_flux = total * fraction;
        let mut allocated = 0.0;
        let last = stars.len() - 1;
        for (index, star) in stars.iter_mut().enumerate() {
            check_cancelled(cancelled)?;
            star.flux = if index == last {
                resolved_flux - allocated
            } else {
                resolved_flux * (star.flux / weight_sum)
            };
            allocated += star.flux;
        }
    }
    let mut pixels = buffer(count, "pixels")?;
    for rgb in radiance {
        check_cancelled(cancelled)?;
        pixels.push(rgb.map(|v| (v * (1.0 - fraction)) as f32));
    }
    check_cancelled(cancelled)?;
    Ok(GalaxyBackground {
        width: config.width,
        height: config.height,
        pixels,
        stars,
    })
}

fn apparent_flux_weight(rank: f64) -> f64 {
    // A finite count prior dN/dm proportional to 10^(0.3m), not a measured
    // Galactic luminosity function. Sampling in magnitude avoids a bright
    // minimum flux shared by almost every packet. Ten magnitudes span 10,000x.
    let magnitude = (1.0 + rank * (10.0_f64.powf(3.0) - 1.0)).log10() / 0.3;
    10.0_f64.powf(-0.4 * magnitude)
}

fn validate(c: &GalaxyConfig) -> Result<(), GalaxyError> {
    for (field, value, minimum, maximum) in [
        ("width", f64::from(c.width), 1.0, 2048.0),
        ("height", f64::from(c.height), 1.0, 2048.0),
        (
            "integration_steps",
            f64::from(c.integration_steps),
            4.0,
            256.0,
        ),
        (
            "star_count",
            f64::from(c.star_count),
            0.0,
            f64::from(MAX_STARS),
        ),
        ("maximum_distance_kpc", c.maximum_distance_kpc, 0.1, 50.0),
        ("resolved_fraction", c.resolved_fraction, 0.0, 1.0),
        ("observer_kpc.x", c.observer_kpc[0], -30.0, 30.0),
        ("observer_kpc.y", c.observer_kpc[1], -30.0, 30.0),
        ("observer_kpc.z", c.observer_kpc[2], -30.0, 30.0),
    ] {
        if !value.is_finite() || value < minimum || value > maximum {
            return Err(GalaxyError::InvalidSetting {
                field,
                value,
                minimum,
                maximum,
            });
        }
    }
    let pixels = u64::from(c.width) * u64::from(c.height);
    for (budget, requested, maximum) in [
        ("pixels", pixels, MAX_PIXELS),
        (
            "ray samples",
            pixels * u64::from(c.integration_steps),
            MAX_RAY_SAMPLES,
        ),
    ] {
        if requested > maximum {
            return Err(GalaxyError::BudgetExceeded {
                budget,
                requested,
                maximum,
            });
        }
    }
    Ok(())
}

fn buffer<T>(count: usize, name: &'static str) -> Result<Vec<T>, GalaxyError> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(count)
        .map_err(|_| GalaxyError::AllocationFailed { buffer: name })?;
    Ok(result)
}

fn check_cancelled(flag: &AtomicBool) -> Result<(), GalaxyError> {
    if flag.load(Ordering::Relaxed) {
        Err(GalaxyError::Cancelled)
    } else {
        Ok(())
    }
}

fn mean(rgb: [f64; 3]) -> f64 {
    (rgb[0] + rgb[1] + rgb[2]) / 3.0
}

fn direction(longitude: f64, cosine: f64) -> [f64; 3] {
    let sine = (1.0 - cosine * cosine).max(0.0).sqrt();
    [-longitude.sin() * sine, cosine, -longitude.cos() * sine]
}

fn pixel_direction(c: &GalaxyConfig, x: u32, y: u32) -> [f64; 3] {
    direction(
        TAU * (f64::from(x) + 0.5) / f64::from(c.width),
        (PI * (f64::from(y) + 0.5) / f64::from(c.height)).cos(),
    )
}

fn pixel_area(c: &GalaxyConfig, y: u32) -> f64 {
    TAU / f64::from(c.width)
        * ((PI * f64::from(y) / f64::from(c.height)).cos()
            - (PI * f64::from(y + 1) / f64::from(c.height)).cos())
}

fn to_galactic(d: [f64; 3]) -> [f64; 3] {
    let v = [
        -0.05487556 * d[0] - 0.87343709 * d[2] - 0.48383502 * d[1],
        -0.86766615 * d[0] - 0.19807637 * d[2] + 0.45598378 * d[1],
        0.49410943 * d[0] - 0.44482963 * d[2] + 0.74698224 * d[1],
    ];
    let length = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    v.map(|x| x / length)
}

struct GalaxyModel<'a> {
    config: &'a GalaxyConfig,
    disk_normalization: f64,
    bulge_normalization: f64,
    spectrum: [f64; 3],
    noise_normalization: f64,
}

impl<'a> GalaxyModel<'a> {
    fn new(config: &'a GalaxyConfig) -> Self {
        let disk_normalization = disk(SUN);
        let disk_integral = 4.0
            * PI
            * (0.88 * (8.2_f64 / 2.12).exp() * 2.12_f64.powi(2) * 0.205
                + 0.12 * (8.2_f64 / 3.05).exp() * 3.05_f64.powi(2) * 0.640)
            / disk_normalization;
        let bulge_normalization =
            (0.15 / 0.85) * disk_integral / ((2.0 * PI).powf(1.5) * 1.8 * 0.42 * 0.72);
        let spectrum = [650e-9_f64, 550e-9, 450e-9]
            .map(|w| 1.0 / (w.powi(5) * (1.438776877e-2 / (w * 5000.0)).exp_m1()));
        Self {
            config,
            disk_normalization,
            bulge_normalization,
            spectrum: spectrum.map(|v| v / spectrum[1]),
            noise_normalization: ((1.0 / 12.0) * (26.0_f64 / 35.0).powi(3)).sqrt().recip(),
        }
    }

    fn octaves(&self, p: [f64; 3], scales: [f64; 3], offsets: [[f64; 3]; 3]) -> f64 {
        let sum: f64 = (0..3)
            .map(|i| {
                (noise(
                    std::array::from_fn(|j| p[j] * scales[i] + offsets[i][j]),
                    self.config.seed,
                ) - 0.5)
                    * WEIGHTS[i]
            })
            .sum();
        sum * self.noise_normalization / WEIGHTS.iter().map(|v| v * v).sum::<f64>().sqrt()
    }

    fn integrate(&self, ray: [f64; 3], cancelled: &AtomicBool) -> Result<[f64; 3], GalaxyError> {
        let sigma = 5.0_f64.ln().sqrt(); // ln(1 + (0.4 * Mach 5)^2)
        let projected = lognormal(
            self.octaves(
                ray,
                [10.0, 1.0 / 0.035, 1.0 / 0.012],
                [OFFSETS[2], OFFSETS[0], OFFSETS[1]],
            ),
            sigma * 0.35_f64.sqrt(),
        );
        let mottle: f64 = (0..2)
            .map(|i| {
                let scale = [10.0, 1.0 / 0.035][i];
                (noise(
                    std::array::from_fn(|j| ray[j] * scale + OFFSETS[i][j]),
                    self.config.seed,
                ) - 0.5)
                    * self.noise_normalization
                    * [0.20, 0.11][i]
            })
            .sum();
        let mottle = (1.0 + mottle).clamp(0.25, 1.95);
        let mut extinction = 0.0;
        let mut result = [0.0; 3];
        let steps = f64::from(self.config.integration_steps);
        for i in 0..self.config.integration_steps {
            if i % 8 == 0 {
                check_cancelled(cancelled)?;
            }
            let start = (f64::from(i) / steps).powi(2) * self.config.maximum_distance_kpc;
            let end = (f64::from(i + 1) / steps).powi(2) * self.config.maximum_distance_kpc;
            let ds = end - start;
            let p =
                std::array::from_fn(|j| self.config.observer_kpc[j] + ray[j] * (start + end) * 0.5);
            let clouds = lognormal(
                self.octaves(p, [0.55, 1.4, 3.7], OFFSETS),
                sigma * 0.65_f64.sqrt(),
            );
            let delta_av = dust(p) * clouds * projected * ds * 0.7;
            let emission =
                (disk(p) / self.disk_normalization * mottle + self.bulge(p)) * ds * EMISSIVITY;
            let attenuation = transmission(extinction + delta_av * 0.5);
            for channel in 0..3 {
                result[channel] += emission * self.spectrum[channel] * attenuation[channel];
            }
            extinction += delta_av;
        }
        Ok(result)
    }

    fn bulge(&self, p: [f64; 3]) -> f64 {
        let (s, c) = 0.47_f64.sin_cos();
        let radius_sq = ((p[0] * c + p[2] * s) / 1.8).powi(2)
            + (p[1] / 0.42).powi(2)
            + ((-p[0] * s + p[2] * c) / 0.72).powi(2);
        self.bulge_normalization * (-0.5 * radius_sq).exp()
    }
}

fn disk(p: [f64; 3]) -> f64 {
    let r = p[0].hypot(p[2]);
    0.88 * ((8.2 - r) / 2.12 - p[1].abs() / 0.205).exp()
        + 0.12 * ((8.2 - r) / 3.05 - p[1].abs() / 0.640).exp()
}

fn dust(p: [f64; 3]) -> f64 {
    ((8.2 - p[0].hypot(p[2])) / 3.192 + (SUN[1].abs() - p[1].abs()) / 0.103).exp()
}

fn lognormal(normal: f64, sigma: f64) -> f64 {
    (sigma * normal - 0.5 * sigma * sigma).exp()
}

fn transmission(visual_extinction_mag: f64) -> [f64; 3] {
    EXTINCTION.map(|coefficient| (-0.921034 * visual_extinction_mag * coefficient).exp())
}

// Seeded integer lattice hashing avoids the entropy loss of float fract hashes.
fn hash(mut v: u64) -> u64 {
    v = (v ^ (v >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    v = (v ^ (v >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    v ^ (v >> 31)
}

fn unit(v: u64) -> f64 {
    (v >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
}

struct Random(u64);
impl Random {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        unit(hash(self.0))
    }
}

fn noise(p: [f64; 3], seed: u64) -> f64 {
    let cell = p.map(|v| v.floor() as i64);
    let u = std::array::from_fn::<_, 3, _>(|i| {
        let f = p[i] - p[i].floor();
        f * f * (3.0 - 2.0 * f)
    });
    let mut result = 0.0;
    for corner in 0..8 {
        let mut h = seed;
        let mut weight = 1.0;
        for axis in 0..3 {
            let bit = (corner >> axis) & 1;
            h = hash(h ^ (cell[axis] + bit) as u64);
            weight *= if bit == 0 { 1.0 - u[axis] } else { u[axis] };
        }
        result += unit(h) * weight;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> GalaxyConfig {
        GalaxyConfig {
            width: 32,
            height: 16,
            integration_steps: 32,
            star_count: 2000,
            ..GalaxyConfig::default()
        }
    }

    fn generate(c: &GalaxyConfig) -> GalaxyBackground {
        generate_background(c, &AtomicBool::new(false)).unwrap()
    }

    fn integrated_mean(sky: &GalaxyBackground) -> f64 {
        let c = GalaxyConfig {
            width: sky.width,
            height: sky.height,
            ..small()
        };
        sky.pixels
            .iter()
            .enumerate()
            .map(|(i, rgb)| mean(rgb.map(f64::from)) * pixel_area(&c, i as u32 / c.width))
            .sum()
    }

    #[test]
    fn seeded_generation_is_repeatable_and_seed_changes_both_layers() {
        let mut c = small();
        let first = generate(&c);
        assert_eq!(first, generate(&c));
        c.seed = u64::MAX;
        let second = generate(&c);
        assert_ne!(first.pixels, second.pixels);
        assert_ne!(first.stars, second.stars);
    }

    #[test]
    fn outputs_are_finite_nonnegative_hdr_and_unit_directions() {
        let c = small();
        let sky = generate(&c);
        assert_eq!((sky.width, sky.height), (c.width, c.height));
        assert_eq!(sky.pixels.len(), (c.width * c.height) as usize);
        assert_eq!(sky.stars.len(), c.star_count as usize);
        assert!(sky
            .pixels
            .iter()
            .flatten()
            .all(|v| v.is_finite() && *v >= 0.0));
        assert!(sky.pixels.iter().flatten().any(|v| *v > 0.0));
        for star in sky.stars {
            assert!(star.flux.is_finite() && star.flux > 0.0);
            assert!(star.color.iter().all(|v| v.is_finite() && *v >= 0.0));
            assert!((star.direction.iter().map(|v| v * v).sum::<f64>() - 1.0).abs() < 1e-12);
            assert!((mean(star.color.map(f64::from)) - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn cancellation_returns_no_partial_background() {
        assert_eq!(
            generate_background(&small(), &AtomicBool::new(true)),
            Err(GalaxyError::Cancelled)
        );
        let flag = AtomicBool::new(true);
        assert_eq!(
            GalaxyModel::new(&small()).integrate([1.0, 0.0, 0.0], &flag),
            Err(GalaxyError::Cancelled)
        );
    }

    #[test]
    fn invalid_settings_and_combined_work_are_bounded_before_allocation() {
        let invalid = [
            GalaxyConfig {
                width: 0,
                ..small()
            },
            GalaxyConfig {
                height: u32::MAX,
                ..small()
            },
            GalaxyConfig {
                integration_steps: 0,
                ..small()
            },
            GalaxyConfig {
                integration_steps: 257,
                ..small()
            },
            GalaxyConfig {
                star_count: MAX_STARS + 1,
                ..small()
            },
            GalaxyConfig {
                resolved_fraction: f64::NAN,
                ..small()
            },
            GalaxyConfig {
                resolved_fraction: -0.1,
                ..small()
            },
            GalaxyConfig {
                resolved_fraction: 1.1,
                ..small()
            },
            GalaxyConfig {
                maximum_distance_kpc: f64::INFINITY,
                ..small()
            },
            GalaxyConfig {
                observer_kpc: [31.0, 0.0, 0.0],
                ..small()
            },
            GalaxyConfig {
                observer_kpc: [0.0, f64::NAN, 0.0],
                ..small()
            },
            GalaxyConfig {
                width: 2048,
                height: 2048,
                ..small()
            },
            GalaxyConfig {
                width: 1024,
                height: 1024,
                integration_steps: 256,
                ..small()
            },
        ];
        for c in invalid {
            assert!(generate_background(&c, &AtomicBool::new(false)).is_err());
        }
        assert!(validate(&GalaxyConfig::default()).is_ok());
        assert!(validate(&GalaxyConfig {
            seed: u64::MAX,
            ..small()
        })
        .is_ok());
    }

    #[test]
    fn resolved_and_unresolved_light_partition_without_double_counting() {
        let c = small();
        let full = generate(&GalaxyConfig {
            resolved_fraction: 0.0,
            ..c.clone()
        });
        let split = generate(&c);
        let total = integrated_mean(&full);
        let sprite_flux: f64 = split.stars.iter().map(|s| s.flux).sum();
        assert!((integrated_mean(&split) / total - (1.0 - c.resolved_fraction)).abs() < 1e-7);
        assert!((sprite_flux / total - c.resolved_fraction).abs() < 1e-7);
        assert!(((integrated_mean(&split) + sprite_flux) / total - 1.0).abs() < 1e-7);
        let different_budget = generate(&GalaxyConfig {
            star_count: 100,
            ..c.clone()
        });
        assert_eq!(split.pixels, different_budget.pixels);
        assert!(
            (different_budget.stars.iter().map(|s| s.flux).sum::<f64>() / sprite_flux - 1.0).abs()
                < 1e-12
        );
        let no_stars = generate(&GalaxyConfig {
            star_count: 0,
            ..c.clone()
        });
        assert_eq!(no_stars.pixels, full.pixels);
        assert!(no_stars.stars.is_empty());
        assert!(full.stars.is_empty());
        let all_stars = generate(&GalaxyConfig {
            resolved_fraction: 1.0,
            ..c
        });
        assert!(all_stars.pixels.iter().flatten().all(|v| *v == 0.0));
    }

    #[test]
    fn equirectangular_orientation_and_solid_angle_match_godot() {
        for (u, expected) in [
            (0.0, [0.0, 0.0, -1.0]),
            (0.25, [-1.0, 0.0, 0.0]),
            (0.5, [0.0, 0.0, 1.0]),
            (0.75, [1.0, 0.0, 0.0]),
        ] {
            let d = direction(TAU * u, 0.0);
            for i in 0..3 {
                assert!((d[i] - expected[i]).abs() < 1e-12);
            }
            let inverse = (-d[0]).atan2(-d[2]).rem_euclid(TAU) / TAU;
            assert!((inverse - u).abs() < 1e-12);
        }
        let c = small();
        let area: f64 = (0..c.height)
            .map(|y| pixel_area(&c, y) * f64::from(c.width))
            .sum();
        assert!((area - 4.0 * PI).abs() < 1e-12);
        assert!(pixel_direction(&c, 0, 0)[1] > 0.0);
        assert!(pixel_direction(&c, 0, c.height - 1)[1] < 0.0);
    }

    #[test]
    fn physical_profiles_are_locally_normalized_and_extinction_reddens() {
        let c = small();
        let m = GalaxyModel::new(&c);
        assert!((disk(SUN) / m.disk_normalization - 1.0).abs() < 1e-14);
        assert!((dust(SUN) - 1.0).abs() < 1e-14);
        assert!(disk([0.0; 3]) > disk(SUN));
        assert!(dust([SUN[0], 1.0, 0.0]) < dust(SUN));
        assert!(m.bulge([0.0; 3]) > m.bulge(SUN));
        let transmission = EXTINCTION.map(|v| (-0.921034 * v).exp());
        assert!(transmission[0] > transmission[1] && transmission[1] > transmission[2]);
        assert_eq!(lognormal(0.0, 0.0), 1.0);
        assert!(noise([0.3, -0.1, 4.2], 0) >= 0.0);
    }

    #[test]
    fn disk_and_dust_obey_their_radial_and_vertical_scale_lengths() {
        let thin = 0.88 * (-SUN[1] / 0.205).exp();
        let thick = 0.12 * (-SUN[1] / 0.640).exp();
        let expected_radial =
            (thin * (-1.0_f64).exp() + thick * (-2.12_f64 / 3.05).exp()) / (thin + thick);
        let expected_vertical =
            (thin * (-1.0_f64).exp() + thick * (-0.205_f64 / 0.640).exp()) / (thin + thick);
        assert!((disk([SUN[0] - 2.12, SUN[1], 0.0]) / disk(SUN) - expected_radial).abs() < 1e-12);
        assert!(
            (disk([SUN[0], SUN[1] + 0.205, 0.0]) / disk(SUN) - expected_vertical).abs() < 1e-12
        );
        assert!((dust([SUN[0] - 3.192, SUN[1], 0.0]) - (-1.0_f64).exp()).abs() < 1e-12);
        assert!((dust([SUN[0], SUN[1] + 0.103, 0.0]) - (-1.0_f64).exp()).abs() < 1e-12);
    }

    #[test]
    fn magnitude_extinction_has_the_logarithmic_intensity_ratio() {
        for (actual, coefficient) in transmission(1.0).into_iter().zip(EXTINCTION) {
            assert!((actual - 10.0_f64.powf(-0.4 * coefficient)).abs() < 1e-7);
        }
        assert_eq!(transmission(0.0), [1.0; 3]);
        let once = transmission(0.5);
        let twice = transmission(1.0);
        for i in 0..3 {
            assert!((once[i] * once[i] - twice[i]).abs() < 1e-14);
        }
    }

    #[test]
    fn smooth_vertical_dust_column_matches_analytic_exponential_integral() {
        let distance = 0.1;
        let expected = 0.7 * 0.103 * (1.0 - (-distance / 0.103_f64).exp());
        let ds = distance / 1024.0;
        let actual: f64 = (0..1024)
            .map(|i| dust([SUN[0], SUN[1] + (f64::from(i) + 0.5) * ds, 0.0]) * ds * 0.7)
            .sum();
        assert!((actual - expected).abs() < 1e-8);
    }

    #[test]
    fn bar_mass_fraction_and_turbulence_variance_match_shared_model() {
        let c = small();
        let model = GalaxyModel::new(&c);
        let disk_mass = 4.0
            * PI
            * (0.88 * (8.2_f64 / 2.12).exp() * 2.12_f64.powi(2) * 0.205
                + 0.12 * (8.2_f64 / 3.05).exp() * 3.05_f64.powi(2) * 0.640)
            / disk(SUN);
        let bar_mass = model.bulge_normalization * (2.0 * PI).powf(1.5) * 1.8 * 0.42 * 0.72;
        assert!((bar_mass / (bar_mass + disk_mass) - 0.15).abs() < 1e-14);
        let variance = (1.0 / 12.0) * (26.0_f64 / 35.0).powi(3);
        assert!((variance * model.noise_normalization.powi(2) - 1.0).abs() < 1e-14);
        assert!((WEIGHTS.iter().sum::<f64>() - 1.0).abs() < 1e-14);
        let sigma_sq = 5.0_f64.ln();
        assert!((sigma_sq * 0.65 + sigma_sq * 0.35 - sigma_sq).abs() < 1e-14);
    }

    #[test]
    fn equatorial_directions_map_to_galactic_center_and_north_pole() {
        for (ra, dec, expected) in [
            (266.4051_f64, -28.936175_f64, [1.0, 0.0, 0.0]),
            (192.85948_f64, 27.12825_f64, [0.0, 1.0, 0.0]),
        ] {
            let (sr, cr) = ra.to_radians().sin_cos();
            let (sd, cd) = dec.to_radians().sin_cos();
            let actual = to_galactic([cd * cr, sd, cd * sr]);
            for i in 0..3 {
                assert!((actual[i] - expected[i]).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn independent_bounded_brightness_weights_preserve_expected_rgb_partition() {
        let c = GalaxyConfig {
            width: 16,
            height: 8,
            star_count: MAX_STARS,
            ..small()
        };
        let full = generate(&GalaxyConfig {
            resolved_fraction: 0.0,
            ..c.clone()
        });
        let split = generate(&c);
        let mut expected = [0.0; 3];
        let mut actual = [0.0; 3];
        for (i, rgb) in full.pixels.iter().enumerate() {
            for channel in 0..3 {
                expected[channel] += f64::from(rgb[channel])
                    * pixel_area(&c, i as u32 / c.width)
                    * c.resolved_fraction;
            }
        }
        for star in &split.stars {
            for (channel, value) in actual.iter_mut().enumerate() {
                *value += f64::from(star.color[channel]) * star.flux;
            }
        }
        for channel in 0..3 {
            assert!((actual[channel] / expected[channel] - 1.0).abs() < 0.02);
        }
        let minimum = split
            .stars
            .iter()
            .map(|s| s.flux)
            .fold(f64::INFINITY, f64::min);
        let maximum = split.stars.iter().map(|s| s.flux).fold(0.0, f64::max);
        assert!(maximum / minimum > 1000.0);
        assert!(maximum / minimum <= 10000.000001);
    }

    #[test]
    fn most_stars_are_faint_with_a_sparse_bright_tail() {
        let sky = generate(&small());
        let mean = sky.stars.iter().map(|s| s.flux).sum::<f64>() / sky.stars.len() as f64;
        let faint = sky.stars.iter().filter(|s| s.flux < mean * 0.2).count();
        let bright = sky.stars.iter().filter(|s| s.flux > mean * 5.0).count();
        assert!(faint > sky.stars.len() / 2);
        assert!(bright > 0 && bright < sky.stars.len() / 20);
        assert!((apparent_flux_weight(0.0) / apparent_flux_weight(1.0) - 10000.0).abs() < 1e-8);
    }

    #[test]
    fn valid_extreme_observers_and_minimum_dimensions_remain_finite() {
        for observer_kpc in [[0.0; 3], [-30.0; 3], [30.0; 3]] {
            let sky = generate(&GalaxyConfig {
                width: 1,
                height: 1,
                integration_steps: 4,
                maximum_distance_kpc: 50.0,
                observer_kpc,
                ..small()
            });
            assert!(sky
                .pixels
                .iter()
                .flatten()
                .all(|v| v.is_finite() && *v >= 0.0));
            assert!(sky
                .stars
                .iter()
                .all(|s| s.flux.is_finite() && s.flux >= 0.0));
        }
    }

    #[test]
    #[ignore = "release timing: cargo test -p dawn-client-core --release galaxy::tests::default_generation_timing -- --ignored --nocapture"]
    fn default_generation_timing() {
        let start = std::time::Instant::now();
        let sky = generate(&GalaxyConfig::default());
        eprintln!(
            "galaxy 512x256, 64 steps, {} stars: {:?}",
            sky.stars.len(),
            start.elapsed()
        );
        eprintln!(
            "continuum integral {}, star flux max {}",
            integrated_mean(&sky),
            sky.stars.iter().map(|s| s.flux).fold(0.0_f64, f64::max)
        );
        assert_eq!(sky.pixels.len(), 512 * 256);
    }
}
