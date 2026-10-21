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

//! The media side of a call: a WebRTC peer connection carrying one Opus audio stream.
//!
//! Audio comes in and goes out as 20 ms frames of 48 kHz mono `i16` samples, so the engine
//! is independent of the sound card (tests feed it synthetic audio). Signalling (SDP and
//! ICE candidates) is exchanged by the caller; see `calls::signalling` for the Matrix side.

use std::sync::{Arc, Mutex as StdMutex};

use bytes::Bytes;
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::RTCConfigurationBuilder;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::{MIME_TYPE_OPUS, MediaEngine};
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::transport::{RTCIceCandidateInit, RTCIceServer};
use rtc::rtp::header::Header;
use rtc::rtp::packet::Packet;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
    RtpCodecKind,
};
use thiserror::Error;
use tokio::sync::mpsc;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCPeerConnectionIceEvent,
    RTCPeerConnectionState,
};
use webrtc::runtime::TokioRuntime;

/// Samples per 20 ms frame at 48 kHz.
pub const FRAME_SAMPLES: usize = 960;
const OPUS_PAYLOAD_TYPE: u8 = 111;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("call setup failed: {0}")]
    Setup(String),
}

fn setup<E: std::fmt::Display>(e: E) -> EngineError {
    EngineError::Setup(e.to_string())
}

/// A TURN or STUN server offered by the homeserver.
#[derive(Debug, Clone, Default)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: String,
    pub credential: String,
}

#[derive(Debug)]
pub enum EngineEvent {
    /// A local ICE candidate to send to the other side.
    Candidate(RTCIceCandidateInit),
    /// Media is flowing.
    Connected,
    /// The connection could not be made or was lost.
    Failed,
    /// The other side went away cleanly.
    Closed,
}

/// Where the call's audio comes from and goes to.
pub struct AudioIo {
    /// Frames of [`FRAME_SAMPLES`] mono samples from the microphone.
    pub mic: mpsc::Receiver<Vec<i16>>,
    /// Decoded frames from the other side, for the speaker.
    pub speaker: mpsc::Sender<Vec<i16>>,
}

struct Handler {
    events: mpsc::UnboundedSender<EngineEvent>,
    speaker: StdMutex<Option<mpsc::Sender<Vec<i16>>>>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_candidate(&self, event: RTCPeerConnectionIceEvent) {
        if let Ok(init) = event.candidate.to_json() {
            let _ = self.events.send(EngineEvent::Candidate(init));
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        let ev = match state {
            RTCPeerConnectionState::Connected => EngineEvent::Connected,
            RTCPeerConnectionState::Failed => EngineEvent::Failed,
            RTCPeerConnectionState::Closed | RTCPeerConnectionState::Disconnected => {
                EngineEvent::Closed
            }
            _ => return,
        };
        let _ = self.events.send(ev);
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let Some(speaker) = self.speaker.lock().expect("speaker lock").take() else {
            return;
        };
        tokio::spawn(async move {
            let Ok(mut decoder) = opus::Decoder::new(48_000, opus::Channels::Mono) else {
                return;
            };
            let mut pcm = vec![0i16; FRAME_SAMPLES * 6];
            while let Some(evt) = track.poll().await {
                if let TrackRemoteEvent::OnRtpPacket(p) = evt {
                    if p.payload.is_empty() {
                        continue;
                    }
                    if let Ok(n) = decoder.decode(&p.payload, &mut pcm, false) {
                        // A full speaker queue means we are behind: drop, don't build delay.
                        let _ = speaker.try_send(pcm[..n].to_vec());
                    }
                }
            }
        });
    }
}

pub struct Engine {
    pc: Arc<dyn PeerConnection>,
    muted: Arc<std::sync::atomic::AtomicBool>,
    mic_task: tokio::task::JoinHandle<()>,
}

fn opus_codec() -> RTCRtpCodec {
    RTCRtpCodec {
        mime_type: MIME_TYPE_OPUS.to_owned(),
        clock_rate: 48_000,
        channels: 2,
        sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
        rtcp_feedback: vec![],
    }
}

/// The address to bind: the interface the default route uses, else loopback.
fn local_addr() -> String {
    let ip = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").map(|_| s))
        .and_then(|s| s.local_addr())
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into());
    format!("{ip}:0")
}

impl Engine {
    /// Build a peer connection that sends the microphone and plays what it receives.
    pub async fn new(
        ice: Vec<IceServer>,
        audio: AudioIo,
    ) -> Result<(Self, mpsc::UnboundedReceiver<EngineEvent>), EngineError> {
        let mut media = MediaEngine::default();
        media
            .register_codec(
                RTCRtpCodecParameters {
                    rtp_codec: opus_codec(),
                    payload_type: OPUS_PAYLOAD_TYPE,
                },
                RtpCodecKind::Audio,
            )
            .map_err(setup)?;
        let registry = register_default_interceptors(rtc::interceptor::Registry::new(), &mut media)
            .map_err(setup)?;
        let config = RTCConfigurationBuilder::new()
            .with_ice_servers(
                ice.into_iter()
                    .map(|s| RTCIceServer {
                        urls: s.urls,
                        username: s.username,
                        credential: s.credential,
                    })
                    .collect(),
            )
            .build();

        let (events, events_rx) = mpsc::unbounded_channel();
        let handler = Arc::new(Handler {
            events,
            speaker: StdMutex::new(Some(audio.speaker)),
        });
        let pc = PeerConnectionBuilder::new()
            .with_configuration(config)
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .with_handler(handler)
            .with_runtime(Arc::new(TokioRuntime))
            .with_udp_addrs(vec![local_addr()])
            .build()
            .await
            .map_err(setup)?;
        let pc: Arc<dyn PeerConnection> = Arc::new(pc);

        let ssrc = rand_u32();
        let track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            "msm-stream".to_owned(),
            "msm-audio".to_owned(),
            "msm-audio".to_owned(),
            RtpCodecKind::Audio,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(ssrc),
                    ..Default::default()
                },
                codec: opus_codec(),
                ..Default::default()
            }],
        )));
        pc.add_track(track.clone() as Arc<dyn TrackLocal>)
            .await
            .map_err(setup)?;

        let muted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mic_task = tokio::spawn(send_microphone(track, audio.mic, ssrc, muted.clone()));
        Ok((
            Self {
                pc,
                muted,
                mic_task,
            },
            events_rx,
        ))
    }

    /// Our offer (SDP) to send to the other side. Local candidates start arriving as events.
    pub async fn create_offer(&self) -> Result<String, EngineError> {
        let offer = self.pc.create_offer(None).await.map_err(setup)?;
        let sdp = offer.sdp.clone();
        self.pc.set_local_description(offer).await.map_err(setup)?;
        Ok(sdp)
    }

    /// Take their offer and produce our answer.
    pub async fn accept_offer(&self, sdp: &str) -> Result<String, EngineError> {
        let offer = RTCSessionDescription::offer(sdp.to_owned()).map_err(setup)?;
        self.pc.set_remote_description(offer).await.map_err(setup)?;
        let answer = self.pc.create_answer(None).await.map_err(setup)?;
        let sdp = answer.sdp.clone();
        self.pc.set_local_description(answer).await.map_err(setup)?;
        Ok(sdp)
    }

    pub async fn set_answer(&self, sdp: &str) -> Result<(), EngineError> {
        let answer = RTCSessionDescription::answer(sdp.to_owned()).map_err(setup)?;
        self.pc.set_remote_description(answer).await.map_err(setup)
    }

    pub async fn add_candidate(&self, c: RTCIceCandidateInit) -> Result<(), EngineError> {
        self.pc.add_ice_candidate(c).await.map_err(setup)
    }

    /// Stop sending our microphone (the other side hears silence).
    pub fn set_muted(&self, muted: bool) {
        self.muted
            .store(muted, std::sync::atomic::Ordering::Relaxed);
    }

    pub async fn close(&self) {
        self.mic_task.abort();
        let _ = self.pc.close().await;
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.mic_task.abort();
    }
}

fn rand_u32() -> u32 {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    u32::from_le_bytes(b)
}

async fn send_microphone(
    track: Arc<TrackLocalStaticRTP>,
    mut mic: mpsc::Receiver<Vec<i16>>,
    ssrc: u32,
    muted: Arc<std::sync::atomic::AtomicBool>,
) {
    let Ok(mut encoder) = opus::Encoder::new(48_000, opus::Channels::Mono, opus::Application::Voip)
    else {
        return;
    };
    let mut seq = (rand_u32() & 0xffff) as u16;
    let mut ts = rand_u32();
    let mut out = vec![0u8; 1500];
    while let Some(frame) = mic.recv().await {
        ts = ts.wrapping_add(frame.len() as u32);
        if muted.load(std::sync::atomic::Ordering::Relaxed) || frame.len() != FRAME_SAMPLES {
            continue;
        }
        let Ok(n) = encoder.encode(&frame, &mut out) else {
            continue;
        };
        seq = seq.wrapping_add(1);
        let packet = Packet {
            header: Header {
                version: 2,
                payload_type: OPUS_PAYLOAD_TYPE,
                sequence_number: seq,
                timestamp: ts,
                ssrc,
                ..Default::default()
            },
            payload: Bytes::copy_from_slice(&out[..n]),
        };
        // Not connected yet: the write fails until a transport exists; just keep going.
        let _ = track.write_rtp(packet).await;
    }
}
