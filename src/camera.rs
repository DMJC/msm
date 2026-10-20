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

//! Reading a QR code from a webcam, for verification by scanning.
//!
//! Frames come from `ffmpeg` (V4L2 input, raw grayscale on stdout) so the app does
//! not link a capture stack; [`decode_frame`] turns a frame into the QR's raw bytes.
//! Matrix verification QR codes are binary, so the payload is returned as bytes.

use std::process::Stdio;
use std::time::Duration;

use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const WIDTH: usize = 640;
const HEIGHT: usize = 480;

#[derive(Debug, Error)]
pub enum CameraError {
    #[error("no camera found (looked for {0})")]
    NoCamera(String),
    #[error("ffmpeg is needed to use the camera but could not be started: {0}")]
    NoFfmpeg(String),
    #[error("the camera produced no picture: {0}")]
    NoPicture(String),
    #[error("no QR code was seen before the time limit")]
    TimedOut,
}

/// The camera to use: `MSM_CAMERA` if set, otherwise the first `/dev/video*` that works.
pub fn candidate_devices() -> Vec<String> {
    if let Ok(d) = std::env::var("MSM_CAMERA")
        && !d.is_empty()
    {
        return vec![d];
    }
    let mut v: Vec<String> = std::fs::read_dir("/dev")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("video"))
        .map(|n| format!("/dev/{n}"))
        .collect();
    v.sort_by_key(|p| {
        p.trim_start_matches("/dev/video")
            .parse::<u32>()
            .unwrap_or(u32::MAX)
    });
    v
}

/// Find a QR code in one 8-bit grayscale frame and return its raw payload.
pub fn decode_frame(width: usize, height: usize, gray: &[u8]) -> Option<Vec<u8>> {
    if gray.len() < width * height {
        return None;
    }
    let mut img =
        rqrr::PreparedImage::prepare_from_greyscale(width, height, |x, y| gray[y * width + x]);
    for grid in img.detect_grids() {
        let mut out = Vec::new();
        if grid.decode_to(&mut out).is_ok() && !out.is_empty() {
            return Some(out);
        }
    }
    None
}

/// Watch the camera until a QR code is read, up to `limit`.
pub async fn scan_qr(limit: Duration) -> Result<Vec<u8>, CameraError> {
    let devices = candidate_devices();
    if devices.is_empty() {
        return Err(CameraError::NoCamera("/dev/video*".into()));
    }
    let mut last = CameraError::NoCamera(devices.join(", "));
    let deadline = tokio::time::Instant::now() + limit;
    for dev in devices {
        match tokio::time::timeout_at(deadline, scan_device(&dev)).await {
            Ok(Ok(bytes)) => return Ok(bytes),
            // A device that cannot capture (metadata nodes) fails fast; try the next.
            Ok(Err(e @ CameraError::NoFfmpeg(_))) => return Err(e),
            Ok(Err(e)) => last = e,
            Err(_) => return Err(CameraError::TimedOut),
        }
    }
    Err(last)
}

async fn scan_device(dev: &str) -> Result<Vec<u8>, CameraError> {
    let mut child = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-f", "v4l2", "-video_size"])
        .arg(format!("{WIDTH}x{HEIGHT}"))
        .args([
            "-i",
            dev,
            "-vf",
            "fps=8,scale=640:480,format=gray",
            "-f",
            "rawvideo",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| CameraError::NoFfmpeg(e.to_string()))?;
    let mut out = child.stdout.take().expect("piped");
    let mut buf = vec![0u8; WIDTH * HEIGHT];
    loop {
        if out.read_exact(&mut buf).await.is_err() {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut err).await;
            }
            let err = err.lines().last().unwrap_or("stream ended").to_owned();
            return Err(CameraError::NoPicture(format!("{dev}: {err}")));
        }
        if let Some(bytes) = decode_frame(WIDTH, HEIGHT, &buf) {
            return Ok(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qrcode::{Color, QrCode};

    /// Paint a QR code into a grayscale frame: scaled up, white margin, grey background.
    fn frame_with_qr(payload: &[u8]) -> Vec<u8> {
        let code = QrCode::new(payload).unwrap();
        let (w, colors) = (code.width(), code.to_colors());
        let scale = 6;
        let (ox, oy) = (150, 100);
        let mut f = vec![180u8; WIDTH * HEIGHT];
        for y in oy - 4 * scale..oy + (w + 4) * scale {
            for x in ox - 4 * scale..ox + (w + 4) * scale {
                f[y * WIDTH + x] = 255;
            }
        }
        for (i, c) in colors.iter().enumerate() {
            if *c == Color::Dark {
                let (mx, my) = (i % w, i / w);
                for y in 0..scale {
                    for x in 0..scale {
                        f[(oy + my * scale + y) * WIDTH + ox + mx * scale + x] = 0;
                    }
                }
            }
        }
        f
    }

    #[test]
    fn binary_payload_survives_a_camera_frame() {
        // Matrix QR codes are binary: NUL, high bytes and a MATRIX header.
        let mut payload = b"MATRIX\x02\x02\x00\x05hello".to_vec();
        payload.extend((0u8..=255).step_by(7));
        let frame = frame_with_qr(&payload);
        assert_eq!(decode_frame(WIDTH, HEIGHT, &frame), Some(payload));
    }

    #[test]
    fn short_frame_is_ignored() {
        assert!(decode_frame(WIDTH, HEIGHT, &[0; 10]).is_none());
    }

    #[test]
    fn blank_frame_has_no_code() {
        assert!(decode_frame(WIDTH, HEIGHT, &vec![200; WIDTH * HEIGHT]).is_none());
    }

    /// Needs a webcam: `cargo test -- --ignored camera_delivers_frames`.
    #[tokio::test]
    #[ignore]
    async fn camera_delivers_frames() {
        // Nothing to read in front of the lens, so the scan must run until the limit
        // (not fail with "no picture"), proving frames arrive at the expected size.
        let r = scan_qr(Duration::from_secs(6)).await;
        assert!(matches!(r, Err(CameraError::TimedOut)), "{r:?}");
    }
}
