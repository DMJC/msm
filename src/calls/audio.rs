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

//! The sound card, through PulseAudio/PipeWire's `parec` and `pacat` tools (48 kHz mono
//! 16-bit, 20 ms frames), so the call engine needs no audio library of its own.

use std::process::Stdio;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::engine::{AudioIo, FRAME_SAMPLES};

const FORMAT: [&str; 6] = [
    "--format=s16le",
    "--rate=48000",
    "--channels=1",
    "--client-name=Matrix Messenger",
    "--latency-msec=20",
    "--process-time-msec=20",
];

/// Open the default microphone and speaker. The helper processes die with the returned
/// [`AudioIo`] (when its channels are dropped, the tasks end and the children are killed).
pub fn system_audio() -> Result<AudioIo, String> {
    let missing = |tool: &str, e: std::io::Error| {
        format!("could not start {tool} (install pulseaudio-utils): {e}")
    };
    let mut rec = Command::new("parec")
        .args(FORMAT)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| missing("parec", e))?;
    let mut play = Command::new("pacat")
        .arg("--playback")
        .args(FORMAT)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| missing("pacat", e))?;

    let (mic_tx, mic) = mpsc::channel::<Vec<i16>>(4);
    let (speaker, mut speaker_rx) = mpsc::channel::<Vec<i16>>(8);

    let mut out = rec.stdout.take().ok_or("no microphone stream")?;
    tokio::spawn(async move {
        let _keep = rec;
        let mut bytes = vec![0u8; FRAME_SAMPLES * 2];
        while out.read_exact(&mut bytes).await.is_ok() {
            let (pairs, _) = bytes.as_chunks::<2>();
            let frame = pairs.iter().map(|b| i16::from_le_bytes(*b)).collect();
            // If the engine is behind, drop the frame rather than queue delay.
            if mic_tx.try_send(frame).is_err() && mic_tx.is_closed() {
                break;
            }
        }
    });

    let mut input = play.stdin.take().ok_or("no speaker stream")?;
    tokio::spawn(async move {
        let _keep = play;
        while let Some(frame) = speaker_rx.recv().await {
            let bytes: Vec<u8> = frame.iter().flat_map(|s| s.to_le_bytes()).collect();
            if input.write_all(&bytes).await.is_err() {
                break;
            }
        }
    });
    Ok(AudioIo { mic, speaker })
}
