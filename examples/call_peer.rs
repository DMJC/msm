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

//! A headless call partner for trying voice calls: signs in, then either calls someone or
//! answers the first call, playing a 440 Hz tone. For manual testing against the GUI.
//!
//! `cargo run --example call_peer -- <homeserver> <user> <password> call <@who:server>`
//! `cargo run --example call_peer -- <homeserver> <user> <password> answer`

#![recursion_limit = "256"]

use std::sync::Arc;
use std::time::Duration;

use matrix_sdk::config::SyncSettings;
use msm::calls::engine::{AudioIo, FRAME_SAMPLES};
use msm::calls::session::{CallEvent, Calls};

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 4 {
        eprintln!("usage: call_peer <homeserver> <user> <password> call <@who:server> | answer");
        return;
    }
    let dir = std::env::temp_dir().join(format!("msm-call-peer-{}", std::process::id()));
    let (client, _) = msm::matrix::authentication::login_unsaved(&a[0], &a[1], &a[2], &dir)
        .await
        .expect("login");
    let (tx, rx) = async_channel::unbounded();
    let tone: msm::calls::session::AudioFactory = Arc::new(|| {
        let (mic_tx, mic) = tokio::sync::mpsc::channel(8);
        let (speaker, mut spk) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move { while spk.recv().await.is_some() {} });
        tokio::spawn(async move {
            let mut phase = 0f32;
            let mut tick = tokio::time::interval(Duration::from_millis(20));
            loop {
                tick.tick().await;
                let f: Vec<i16> = (0..FRAME_SAMPLES)
                    .map(|_| {
                        phase += 2.0 * std::f32::consts::PI * 440.0 / 48_000.0;
                        (phase.sin() * 4000.0) as i16
                    })
                    .collect();
                if mic_tx.send(f).await.is_err() {
                    return;
                }
            }
        });
        Ok(AudioIo { mic, speaker })
    });
    let calls = Calls::start(&client, tx, tone);
    let c = client.clone();
    tokio::spawn(async move {
        let _ = c.sync(SyncSettings::default()).await;
    });
    tokio::time::sleep(Duration::from_secs(3)).await;
    if a[3] == "call" {
        let who = &a[4];
        let room = msm::matrix::rooms::open_or_create_dm(&client, who)
            .await
            .expect("dm");
        calls.place(room, who).await.expect("place");
    }
    while let Ok(ev) = rx.recv().await {
        println!("{ev:?}");
        match ev {
            CallEvent::Incoming { .. } if a[3] == "answer" => {
                calls.answer().await.expect("answer");
            }
            CallEvent::Ended { .. } => break,
            _ => {}
        }
    }
}
