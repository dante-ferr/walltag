//! Average-color sampling and hue ordering for the **Color (hue)** sort.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gtk4::gdk_pixbuf::Pixbuf;

/// Thumbnails are decoded to fit in a square of this many pixels before averaging.
const SAMPLE_SIZE: i32 = 32;
const SAMPLE_THREADS: usize = 4;
/// Below this HSL saturation an average color counts as neutral (grayscale-like).
pub const NEUTRAL_SATURATION: f32 = 0.12;

/// Average color of an image in HSL. `hue` is in degrees, `0.0..360.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvgColor {
    pub hue: f32,
    pub saturation: f32,
    pub lightness: f32,
}

impl AvgColor {
    pub fn from_rgb(r: f32, g: f32, b: f32) -> Self {
        let (hue, saturation, lightness) = rgb_to_hsl(r, g, b);
        Self {
            hue,
            saturation,
            lightness,
        }
    }

    pub fn is_neutral(&self) -> bool {
        self.saturation < NEUTRAL_SATURATION
    }
}

/// Converts RGB (each `0.0..=1.0`) to `(hue degrees, saturation, lightness)`.
pub fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.0;
    let delta = max - min;
    if delta <= f32::EPSILON {
        return (0.0, 0.0, lightness);
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs()).max(f32::EPSILON);
    let hue = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    } * 60.0;
    (hue.rem_euclid(360.0), saturation.clamp(0.0, 1.0), lightness)
}

/// Sort key for the hue order. Chromatic images come first by hue around the wheel
/// (red at 0°), neutral images follow by lightness (dark to light), and images with no
/// sampled color come last.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HueKey {
    Chromatic(f32),
    Neutral(f32),
    Unknown,
}

impl HueKey {
    pub fn new(color: Option<&AvgColor>) -> Self {
        match color {
            None => Self::Unknown,
            Some(c) if c.is_neutral() => Self::Neutral(c.lightness),
            Some(c) => Self::Chromatic(c.hue),
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Chromatic(_) => 0,
            Self::Neutral(_) => 1,
            Self::Unknown => 2,
        }
    }

    fn value(self) -> f32 {
        match self {
            Self::Chromatic(v) | Self::Neutral(v) => v,
            Self::Unknown => 0.0,
        }
    }
}

impl PartialOrd for HueKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(
            self.rank()
                .cmp(&other.rank())
                .then_with(|| self.value().total_cmp(&other.value())),
        )
    }
}

/// Hue key for a group of images: the circular mean hue of its chromatic members. A group
/// with no chromatic member uses the mean lightness of its neutral members.
pub fn group_hue_key<'a>(colors: impl IntoIterator<Item = Option<&'a AvgColor>>) -> HueKey {
    let (mut sin, mut cos, mut chromatic) = (0.0f32, 0.0f32, 0u32);
    let (mut light, mut neutral) = (0.0f32, 0u32);
    for color in colors.into_iter().flatten() {
        if color.is_neutral() {
            light += color.lightness;
            neutral += 1;
        } else {
            let rad = color.hue.to_radians();
            sin += rad.sin();
            cos += rad.cos();
            chromatic += 1;
        }
    }
    if chromatic > 0 {
        HueKey::Chromatic(sin.atan2(cos).to_degrees().rem_euclid(360.0))
    } else if neutral > 0 {
        HueKey::Neutral(light / neutral as f32)
    } else {
        HueKey::Unknown
    }
}

/// Averages RGB over a pixel buffer. Transparent pixels weigh less. Returns `None` for
/// an empty or fully transparent buffer.
pub fn average_rgb(
    pixels: &[u8],
    width: usize,
    height: usize,
    rowstride: usize,
    channels: usize,
) -> Option<(f32, f32, f32)> {
    if width == 0 || height == 0 || channels < 3 {
        return None;
    }
    let (mut r, mut g, mut b, mut weight) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for y in 0..height {
        let row = pixels.get(y * rowstride..)?;
        for x in 0..width {
            let px = row.get(x * channels..x * channels + channels)?;
            let a = if channels >= 4 {
                px[3] as f64 / 255.0
            } else {
                1.0
            };
            r += px[0] as f64 * a;
            g += px[1] as f64 * a;
            b += px[2] as f64 * a;
            weight += a;
        }
    }
    if weight <= 0.0 {
        return None;
    }
    let scale = 255.0 * weight;
    Some(((r / scale) as f32, (g / scale) as f32, (b / scale) as f32))
}

/// Decodes a small thumbnail and returns its average color. `None` if the file cannot
/// be decoded.
pub fn sample_color(path: &Path) -> Option<AvgColor> {
    let pixbuf = Pixbuf::from_file_at_scale(path, SAMPLE_SIZE, SAMPLE_SIZE, true).ok()?;
    let bytes = pixbuf.read_pixel_bytes();
    let (r, g, b) = average_rgb(
        &bytes,
        pixbuf.width().max(0) as usize,
        pixbuf.height().max(0) as usize,
        pixbuf.rowstride().max(0) as usize,
        pixbuf.n_channels().max(0) as usize,
    )?;
    Some(AvgColor::from_rgb(r, g, b))
}

/// Samples the average color of each `(key, path)` on worker threads. Files that cannot
/// be decoded are omitted.
pub fn probe_colors(items: Vec<(String, PathBuf)>) -> HashMap<String, AvgColor> {
    if items.is_empty() {
        return HashMap::new();
    }
    let chunk = items.len().div_ceil(SAMPLE_THREADS).max(1);
    let mut out = HashMap::with_capacity(items.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .filter_map(|(key, path)| Some((key.clone(), sample_color(path)?)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for handle in handles {
            if let Ok(found) = handle.join() {
                out.extend(found);
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(r: f32, g: f32, b: f32) -> AvgColor {
        AvgColor::from_rgb(r, g, b)
    }

    #[test]
    fn primary_hues() {
        assert_eq!(rgb_to_hsl(1.0, 0.0, 0.0).0, 0.0);
        assert_eq!(rgb_to_hsl(0.0, 1.0, 0.0).0, 120.0);
        assert_eq!(rgb_to_hsl(0.0, 0.0, 1.0).0, 240.0);
        assert_eq!(rgb_to_hsl(1.0, 1.0, 0.0).0, 60.0);
        // Magenta-ish red stays inside 0..360.
        let h = rgb_to_hsl(1.0, 0.0, 0.2).0;
        assert!((340.0..360.0).contains(&h), "hue {h}");
    }

    #[test]
    fn gray_has_zero_saturation() {
        let (h, s, l) = rgb_to_hsl(0.5, 0.5, 0.5);
        assert_eq!((h, s), (0.0, 0.0));
        assert!((l - 0.5).abs() < 1e-6);
        assert!(color(0.5, 0.5, 0.5).is_neutral());
        assert!(color(0.52, 0.5, 0.5).is_neutral());
        assert!(!color(0.9, 0.2, 0.2).is_neutral());
    }

    #[test]
    fn hue_order_goes_around_wheel_then_neutral_then_unknown() {
        let red = color(1.0, 0.0, 0.0);
        let green = color(0.0, 1.0, 0.0);
        let blue = color(0.0, 0.0, 1.0);
        let dark = color(0.1, 0.1, 0.1);
        let light = color(0.9, 0.9, 0.9);
        let mut keys = vec![
            ("unknown", HueKey::new(None)),
            ("light", HueKey::new(Some(&light))),
            ("blue", HueKey::new(Some(&blue))),
            ("dark", HueKey::new(Some(&dark))),
            ("red", HueKey::new(Some(&red))),
            ("green", HueKey::new(Some(&green))),
        ];
        keys.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        let names: Vec<_> = keys.iter().map(|k| k.0).collect();
        assert_eq!(names, ["red", "green", "blue", "dark", "light", "unknown"]);
    }

    #[test]
    fn group_hue_uses_circular_mean() {
        // 350° and 10° average to 0°, not 180°.
        let a = AvgColor {
            hue: 350.0,
            saturation: 0.8,
            lightness: 0.5,
        };
        let b = AvgColor {
            hue: 10.0,
            saturation: 0.8,
            lightness: 0.5,
        };
        match group_hue_key([Some(&a), Some(&b)]) {
            HueKey::Chromatic(h) => assert!(h < 1.0 || h > 359.0, "hue {h}"),
            other => panic!("unexpected {other:?}"),
        }
        let gray = color(0.4, 0.4, 0.4);
        assert!(matches!(
            group_hue_key([Some(&gray), None]),
            HueKey::Neutral(_)
        ));
        assert_eq!(group_hue_key([None, None]), HueKey::Unknown);
    }

    #[test]
    fn averages_rgb_with_rowstride_and_alpha() {
        // 2x1 RGBA image, rowstride 8: opaque red + fully transparent blue.
        let px = [255, 0, 0, 255, 0, 0, 255, 0];
        let (r, g, b) = average_rgb(&px, 2, 1, 8, 4).unwrap();
        assert!((r - 1.0).abs() < 1e-5 && g == 0.0 && b == 0.0);
        // 2x2 RGB with 2 bytes of row padding.
        let px = [
            255, 0, 0, 0, 0, 255, 9, 9, //
            255, 0, 0, 0, 0, 255, 9, 9,
        ];
        let (r, _, b) = average_rgb(&px, 2, 2, 8, 3).unwrap();
        assert!((r - 0.5).abs() < 1e-5 && (b - 0.5).abs() < 1e-5);
        assert!(average_rgb(&[0, 0, 0, 0], 1, 1, 4, 4).is_none());
        assert!(average_rgb(&[], 1, 1, 4, 3).is_none());
    }
}
