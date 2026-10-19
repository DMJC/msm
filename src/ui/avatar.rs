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
    let pb = Pixbuf::new(Colorspace::Rgb, false, 8, SIZE, SIZE).expect("pixbuf allocation");
    pb.fill(u32::from_be_bytes([r, g, b, 0xff]));
    pb
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
    fn placeholder_is_stable_and_distinct() {
        let a = placeholder("@a:x");
        assert_eq!(a.width(), SIZE);
        assert_eq!(a.read_pixel_bytes(), placeholder("@a:x").read_pixel_bytes());
        assert_ne!(a.read_pixel_bytes(), placeholder("@b:x").read_pixel_bytes());
    }
}
