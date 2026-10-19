//! Procedurally drawn status icons, so the tray and window icons work without
//! any installed icon theme (needed on Wayland panels that only receive pixmaps).

use crate::matrix::presence::UiStatus;

pub fn status_rgb(status: UiStatus) -> (u8, u8, u8) {
    match status {
        UiStatus::Online => (0x3a, 0xb0, 0x3a),
        UiStatus::Away => (0xe8, 0xb0, 0x20),
        UiStatus::Busy => (0xd0, 0x30, 0x30),
        UiStatus::Offline => (0x90, 0x90, 0x90),
    }
}

/// A square RGBA8 image: a white "chat bubble" disc with a coloured status dot.
pub fn status_icon_rgba(status: UiStatus, size: usize) -> Vec<u8> {
    let (r, g, b) = status_rgb(status);
    let s = size as f32;
    let mut px = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let d_main = ((fx - s * 0.5).powi(2) + (fy - s * 0.5).powi(2)).sqrt();
            let rgba = if d_main <= s * 0.34 {
                [r, g, b, 255]
            } else if d_main <= s * 0.47 {
                [0xf4, 0xf4, 0xf4, 255]
            } else {
                [0, 0, 0, 0]
            };
            let i = (y * size + x) * 4;
            px[i..i + 4].copy_from_slice(&rgba);
        }
    }
    px
}

/// ARGB32, network byte order, as the StatusNotifierItem spec requires.
pub fn status_icon_argb(status: UiStatus, size: usize) -> Vec<u8> {
    status_icon_rgba(status, size)
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[3], p[0], p[1], p[2]])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_is_status_coloured_corner_is_transparent() {
        for s in UiStatus::ALL {
            let img = status_icon_rgba(s, 22);
            let c = (11 * 22 + 11) * 4;
            let (r, g, b) = status_rgb(s);
            assert_eq!(&img[c..c + 4], &[r, g, b, 255]);
            assert_eq!(img[3], 0);
        }
    }

    #[test]
    fn statuses_are_visually_distinct() {
        let colours: Vec<_> = UiStatus::ALL.iter().map(|s| status_rgb(*s)).collect();
        for i in 0..colours.len() {
            for j in i + 1..colours.len() {
                assert_ne!(colours[i], colours[j]);
            }
        }
    }

    #[test]
    fn argb_layout() {
        let argb = status_icon_argb(UiStatus::Busy, 4);
        assert_eq!(argb.len(), 4 * 4 * 4);
        let (r, g, b) = status_rgb(UiStatus::Busy);
        let c = (2 * 4 + 2) * 4;
        assert_eq!(&argb[c..c + 4], &[255, r, g, b]);
    }
}
