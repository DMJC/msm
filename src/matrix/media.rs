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

//! Sending and downloading attachments (files, images, audio, video).
//!
//! Encrypted rooms get encrypted uploads automatically (SDK); downloads are
//! decrypted by the SDK. Filenames come from other people and are never used as
//! paths without [`sanitize_filename`].

use std::path::Path;

use matrix_sdk::attachment::{
    AttachmentConfig, AttachmentInfo, BaseAudioInfo, BaseFileInfo, BaseImageInfo, BaseVideoInfo,
};
use matrix_sdk::media::{MediaFormat, MediaRequestParameters};
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::{Client, Room};
use thiserror::Error;

/// Largest file we will upload.
pub const MAX_UPLOAD_BYTES: u64 = 50 * 1024 * 1024;
/// Largest attachment we will download into memory (declared size).
pub const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("could not read the file: {0}")]
    Read(String),
    #[error("the file is too large ({size}, limit {limit})")]
    TooLarge { size: String, limit: String },
    #[error("{0}")]
    Matrix(String),
    #[error("cancelled")]
    Cancelled,
}

/// Reduce an untrusted filename to a safe single path component.
pub fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim().to_owned();
    if cleaned.is_empty() {
        return "file".to_owned();
    }
    // Keep the extension when shortening very long names.
    if cleaned.chars().count() > 120 {
        let (stem, ext) = match cleaned.rsplit_once('.') {
            Some((s, e)) if e.chars().count() <= 10 => (s.to_owned(), format!(".{e}")),
            _ => (cleaned.clone(), String::new()),
        };
        let keep = 120usize.saturating_sub(ext.chars().count());
        return format!("{}{ext}", stem.chars().take(keep).collect::<String>());
    }
    cleaned
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

pub fn guess_mime(path: &Path) -> mime_guess::mime::Mime {
    mime_guess::from_path(path).first_or_octet_stream()
}

fn mx(e: impl std::fmt::Display) -> MediaError {
    MediaError::Matrix(e.to_string())
}

/// Optional observers for an upload: bytes sent so far / total on `progress`,
/// and a message on `cancel` abandons it.
#[derive(Default)]
pub struct Transfer {
    pub progress: Option<async_channel::Sender<(usize, usize)>>,
    pub cancel: Option<async_channel::Receiver<()>>,
}

/// Upload `path` and post it to `room`. Returns the new event's ID.
pub async fn send_file(room: &Room, path: &Path) -> Result<String, MediaError> {
    send_file_with(room, path, Transfer::default()).await
}

pub async fn send_file_with(
    room: &Room,
    path: &Path,
    transfer: Transfer,
) -> Result<String, MediaError> {
    let meta = std::fs::metadata(path).map_err(|e| MediaError::Read(e.to_string()))?;
    if !meta.is_file() {
        return Err(MediaError::Read("not a regular file".into()));
    }
    if meta.len() > MAX_UPLOAD_BYTES {
        return Err(MediaError::TooLarge {
            size: human_size(meta.len()),
            limit: human_size(MAX_UPLOAD_BYTES),
        });
    }
    let data = std::fs::read(path).map_err(|e| MediaError::Read(e.to_string()))?;
    let name = sanitize_filename(
        &path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let mime = guess_mime(path);
    // Declare the size (and kind) so receivers can show it before downloading.
    let size = Some(matrix_sdk::ruma::UInt::from(data.len() as u32));
    let info = match mime.type_().as_str() {
        "image" => AttachmentInfo::Image(BaseImageInfo {
            size,
            ..Default::default()
        }),
        "video" => AttachmentInfo::Video(BaseVideoInfo {
            size,
            ..Default::default()
        }),
        "audio" => AttachmentInfo::Audio(BaseAudioInfo {
            size,
            ..Default::default()
        }),
        _ => AttachmentInfo::File(BaseFileInfo { size }),
    };
    let observable = eyeball::SharedObservable::new(matrix_sdk::TransmissionProgress::default());
    let upload = room
        .send_attachment(name, &mime, data, AttachmentConfig::new().info(info))
        .with_send_progress_observable(observable.clone());
    let reporter = transfer.progress.map(|tx| {
        let mut sub = observable.subscribe();
        async move {
            while let Some(p) = sub.next().await {
                if tx.send((p.current, p.total)).await.is_err() {
                    break;
                }
            }
            // Nobody is listening any more; the upload carries on regardless.
            std::future::pending::<()>().await
        }
    });
    let cancelled = async {
        match &transfer.cancel {
            Some(rx) => {
                let _ = rx.recv().await;
            }
            None => std::future::pending().await,
        }
    };
    let reporter = async {
        match reporter {
            Some(r) => r.await,
            None => std::future::pending().await,
        }
    };
    // Dropping the upload future abandons the request.
    let r = tokio::select! {
        r = upload => r.map_err(mx)?,
        () = cancelled => return Err(MediaError::Cancelled),
        () = reporter => unreachable!("the progress reporter never finishes"),
    };
    Ok(r.event_id.to_string())
}

/// Download (and, for encrypted media, decrypt) an attachment.
pub async fn download(
    client: &Client,
    source: &MediaSource,
    declared_size: Option<u64>,
) -> Result<Vec<u8>, MediaError> {
    if let Some(s) = declared_size
        && s > MAX_DOWNLOAD_BYTES
    {
        return Err(MediaError::TooLarge {
            size: human_size(s),
            limit: human_size(MAX_DOWNLOAD_BYTES),
        });
    }
    client
        .media()
        .get_media_content(
            &MediaRequestParameters {
                source: source.clone(),
                format: MediaFormat::File,
            },
            true,
        )
        .await
        .map_err(mx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_hostile_filenames() {
        assert_eq!(sanitize_filename("report.pdf"), "report.pdf");
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("C:\\Windows\\evil.exe"), "evil.exe");
        assert_eq!(sanitize_filename("/abs/path/file.txt"), "file.txt");
        assert_eq!(sanitize_filename(".bashrc"), "bashrc");
        assert_eq!(sanitize_filename("..."), "file");
        assert_eq!(sanitize_filename(""), "file");
        assert_eq!(sanitize_filename("a<b>c|d?.txt"), "a_b_c_d_.txt");
        assert_eq!(sanitize_filename("line\nbreak\u{0}.txt"), "linebreak.txt");
        assert_eq!(sanitize_filename("日本語 🦀.png"), "日本語 🦀.png");
        assert_eq!(sanitize_filename("dir/"), "file");
    }

    #[test]
    fn long_names_keep_their_extension() {
        let n = format!("{}.tar.gz", "x".repeat(300));
        let s = sanitize_filename(&n);
        assert!(s.chars().count() <= 120);
        assert!(s.ends_with(".gz"));
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn guesses_mime_types() {
        assert_eq!(guess_mime(Path::new("a.png")).to_string(), "image/png");
        assert_eq!(guess_mime(Path::new("a.txt")).type_(), "text");
        assert_eq!(
            guess_mime(Path::new("noext")).to_string(),
            "application/octet-stream"
        );
    }
}
