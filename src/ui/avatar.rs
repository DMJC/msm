// Copyright (C) 2026 James Carthew
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Decoding and placeholders for avatars. Avatar bytes are untrusted: decoding
//! failures yield `None` and the caller shows a placeholder.

use gtk::gdk_pixbuf::prelude::*;
use gtk::gdk_pixbuf::{Colorspace, InterpType, Pixbuf, PixbufLoader};

/// Edge length (px) of every avatar we keep and display.
pub const SIZE: i32 = 40;
/// Refuse to decode absurd dimensions (decompression bombs).
const MAX_DIMENSION: i32 = 8192;

/// Decode, centre-crop to a square and scale to [`SIZE`].
pub fn decode(bytes: &[u8]) -> Option<Pixbuf> {
    let loader = PixbufLoader::new();
    // Reject oversized images before they are fully decoded.
    loader.connect_size_prepared(|l, w, h| {
        if w > MAX_DIMENSION || h > MAX_DIMENSION {
            l.set_size(1, 1);
        }
    });
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    let pb = loader.pixbuf()?;
    let (w, h) = (pb.width(), pb.height());
    if w < 2 && h < 2 {
        return None;
    }
    let side = w.min(h);
    let square = pb.new_subpixbuf((w - side) / 2, (h - side) / 2, side, side);
    square.scale_simple(SIZE, SIZE, InterpType::Bilinear)
}

/// A flat colour derived from `key`, so the same person always looks the same.
pub fn placeholder(key: &str) -> Pixbuf {
    let hash = key.bytes().fold(2166136261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16777619)
    });
    let (r, g, b) = hsv_to_rgb((hash % 360) as f32, 0.45, 0.75);
    let flat = || {
        let pb = Pixbuf::new(Colorspace::Rgb, false, 8, SIZE, SIZE).expect("pixbuf allocation");
        pb.fill(u32::from_be_bytes([r, g, b, 0xff]));
        pb
    };
    let Some(letter) = initial(key) else {
        return flat();
    };
    draw_initial(letter, (r, g, b)).unwrap_or_else(flat)
}

/// First letter or digit of a user key such as `u:@alice:server`, upper-cased.
fn initial(key: &str) -> Option<char> {
    // Room keys carry an opaque ID, so only people get a letter.
    let name = key.strip_prefix("u:")?;
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().next().unwrap_or(c))
}

fn draw_initial(letter: char, (r, g, b): (u8, u8, u8)) -> Option<Pixbuf> {
    use gtk::cairo::{Context, FontSlant, FontWeight, Format, ImageSurface};
    let surface = ImageSurface::create(Format::ARgb32, SIZE, SIZE).ok()?;
    {
        let cr = Context::new(&surface).ok()?;
        let c = |v: u8| f64::from(v) / 255.0;
        cr.set_source_rgb(c(r), c(g), c(b));
        cr.paint().ok()?;
        cr.select_font_face("sans-serif", FontSlant::Normal, FontWeight::Bold);
        cr.set_font_size(f64::from(SIZE) * 0.55);
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let text = letter.to_string();
        let ext = cr.text_extents(&text).ok()?;
        let side = f64::from(SIZE);
        cr.move_to(
            (side - ext.width()) / 2.0 - ext.x_bearing(),
            (side - ext.height()) / 2.0 - ext.y_bearing(),
        );
        cr.show_text(&text).ok()?;
    }
    // The surface is opaque ARGB32 (BGRA bytes on little-endian); repack as RGB.
    let stride = surface.stride() as usize;
    let data = surface.take_data().ok()?;
    let mut rgb = Vec::with_capacity((SIZE * SIZE * 3) as usize);
    for row in 0..SIZE as usize {
        for px in data[row * stride..][..SIZE as usize * 4].as_chunks::<4>().0 {
            let argb = u32::from_ne_bytes(*px);
            rgb.extend([(argb >> 16) as u8, (argb >> 8) as u8, argb as u8]);
        }
    }
    Some(Pixbuf::from_bytes(
        &gtk::glib::Bytes::from_owned(rgb),
        Colorspace::Rgb,
        false,
        8,
        SIZE,
        SIZE,
        SIZE * 3,
    ))
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let f = |n: f32| ((n + m) * 255.0).round() as u8;
    (f(r), f(g), f(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let pb = Pixbuf::new(Colorspace::Rgb, true, 8, w as i32, h as i32).unwrap();
        pb.fill(0xff0000ff);
        pb.save_to_bufferv("png", &[]).unwrap()
    }

    #[test]
    fn decodes_and_squares_images() {
        for (w, h) in [(100, 100), (300, 90), (20, 400)] {
            let pb = decode(&png(w, h)).expect("decodes");
            assert_eq!((pb.width(), pb.height()), (SIZE, SIZE));
        }
    }

    #[test]
    fn garbage_and_empty_data_are_rejected() {
        assert!(decode(b"definitely not an image").is_none());
        assert!(decode(&[]).is_none());
    }

    #[test]
    fn initials_skip_sigils_and_uppercase() {
        assert_eq!(initial("u:@alice:x"), Some('A'));
        assert_eq!(initial("u:@9bob:x"), Some('9'));
        assert_eq!(initial("r:!abc:x"), None);
        assert_eq!(initial(""), None);
        assert_eq!(initial("u:@:"), None);
    }

    #[test]
    fn placeholder_is_stable_and_distinct() {
        let a = placeholder("@a:x");
        assert_eq!(a.width(), SIZE);
        assert_eq!(a.read_pixel_bytes(), placeholder("@a:x").read_pixel_bytes());
        assert_ne!(a.read_pixel_bytes(), placeholder("@b:x").read_pixel_bytes());
    }
}
