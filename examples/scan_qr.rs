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

//! Hold a QR code up to the camera: `cargo run --example scan_qr` (set `MSM_CAMERA` to
//! pick a device). Prints whether a Matrix verification code was read.

use std::time::Duration;

use matrix_sdk::encryption::verification::QrVerificationData;

#[tokio::main]
async fn main() {
    eprintln!("Looking for a QR code for 30 seconds…");
    match msm::camera::scan_qr(Duration::from_secs(30)).await {
        Ok(bytes) => match QrVerificationData::from_bytes(&bytes) {
            Ok(_) => println!(
                "Read a Matrix verification QR code ({} bytes).",
                bytes.len()
            ),
            Err(e) => println!(
                "Read a QR code ({} bytes) but it is not a Matrix one: {e}",
                bytes.len()
            ),
        },
        Err(e) => println!("{e}"),
    }
}
