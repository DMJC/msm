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

//! Two call engines in one process: offer/answer and candidates exchanged directly, a sine
//! wave sent from one side must arrive, decoded, on the other.

use std::time::Duration;

use msm::calls::engine::{AudioIo, Engine, EngineEvent, FRAME_SAMPLES};
use tokio::sync::mpsc;

fn sine_frames(tx: mpsc::Sender<Vec<i16>>) {
    tokio::spawn(async move {
        let mut phase = 0f32;
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        loop {
            tick.tick().await;
            let frame: Vec<i16> = (0..FRAME_SAMPLES)
                .map(|_| {
                    phase += 2.0 * std::f32::consts::PI * 440.0 / 48_000.0;
                    (phase.sin() * 8000.0) as i16
                })
                .collect();
            if tx.send(frame).await.is_err() {
                return;
            }
        }
    });
}

fn peak(frame: &[i16]) -> i16 {
    frame.iter().map(|s| s.abs()).max().unwrap_or(0)
}

#[tokio::test]
async fn audio_flows_between_two_engines() {
    let (a_mic_tx, a_mic) = mpsc::channel(8);
    let (a_spk, mut a_heard) = mpsc::channel(64);
    let (b_mic_tx, b_mic) = mpsc::channel(8);
    let (b_spk, mut b_heard) = mpsc::channel(64);
    sine_frames(a_mic_tx);
    sine_frames(b_mic_tx);

    let (a, mut a_ev) = Engine::new(
        vec![],
        AudioIo {
            mic: a_mic,
            speaker: a_spk,
        },
    )
    .await
    .unwrap();
    let (b, mut b_ev) = Engine::new(
        vec![],
        AudioIo {
            mic: b_mic,
            speaker: b_spk,
        },
    )
    .await
    .unwrap();

    let offer = a.create_offer().await.unwrap();
    assert!(offer.contains("opus/48000/2"), "{offer}");
    let answer = b.accept_offer(&offer).await.unwrap();
    a.set_answer(&answer).await.unwrap();

    // Trickle candidates across until both sides report Connected.
    let (mut a_up, mut b_up) = (false, false);
    let outcome = tokio::time::timeout(Duration::from_secs(20), async {
        while !(a_up && b_up) {
            tokio::select! {
                Some(e) = a_ev.recv() => match e {
                    EngineEvent::Candidate(c) => b.add_candidate(c).await.unwrap(),
                    EngineEvent::Connected => a_up = true,
                    EngineEvent::Failed => panic!("a failed"),
                    EngineEvent::Closed => {}
                },
                Some(e) = b_ev.recv() => match e {
                    EngineEvent::Candidate(c) => a.add_candidate(c).await.unwrap(),
                    EngineEvent::Connected => b_up = true,
                    EngineEvent::Failed => panic!("b failed"),
                    EngineEvent::Closed => {}
                },
            }
        }
    })
    .await;
    assert!(outcome.is_ok(), "never connected (a={a_up}, b={b_up})");

    for heard in [&mut a_heard, &mut b_heard] {
        let got = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let f = heard.recv().await.expect("speaker closed");
                if peak(&f) > 2000 {
                    return f;
                }
            }
        })
        .await
        .expect("no audible audio arrived");
        assert_eq!(got.len(), FRAME_SAMPLES);
    }

    // Muting stops what the other side receives.
    a.set_muted(true);
    tokio::time::sleep(Duration::from_millis(300)).await;
    while b_heard.try_recv().is_ok() {}
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        b_heard.try_recv().is_err(),
        "audio still arriving while muted"
    );

    a.close().await;
    b.close().await;
}
