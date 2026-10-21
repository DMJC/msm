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

//! Matrix call signalling (VoIP version 1, one-to-one) around the media [`Engine`]:
//! `m.call.invite` / `answer` / `select_answer` / `candidates` / `hangup` / `reject`, as
//! used by Element, so msm can call and be called by other clients. Only one call at a time.

use std::sync::Arc;
use std::time::Duration;

use matrix_sdk::Client;
use matrix_sdk::Room;
use matrix_sdk::ruma::events::call::answer::OriginalSyncCallAnswerEvent;
use matrix_sdk::ruma::events::call::candidates::OriginalSyncCallCandidatesEvent;
use matrix_sdk::ruma::events::call::hangup::OriginalSyncCallHangupEvent;
use matrix_sdk::ruma::events::call::invite::OriginalSyncCallInviteEvent;
use matrix_sdk::ruma::events::call::reject::OriginalSyncCallRejectEvent;
use matrix_sdk::ruma::events::call::select_answer::OriginalSyncCallSelectAnswerEvent;
use rtc::peer_connection::transport::RTCIceCandidateInit;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use super::engine::{AudioIo, Engine, EngineEvent, IceServer};

/// How long an invitation stays valid, and how long we ring for.
const RING_MS: u64 = 60_000;
/// Events older than this when first seen are history, not a call to answer.
const MAX_AGE_MS: u64 = 90_000;

/// What the user interface needs to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallEvent {
    /// Someone is calling us.
    Incoming {
        call_id: String,
        room_id: String,
        caller: String,
    },
    /// We called; waiting for them to answer.
    Ringing { call_id: String, room_id: String },
    /// Answered; setting up the media connection.
    Connecting,
    /// Audio is flowing.
    Active,
    /// The call is over (or never started).
    Ended { reason: String },
}

pub type AudioFactory = Arc<dyn Fn() -> Result<AudioIo, String> + Send + Sync>;

struct Call {
    id: String,
    room: Room,
    my_party: String,
    peer: String,
    incoming: bool,
    answered: bool,
    /// Their offer, kept until we answer.
    offer_sdp: Option<String>,
    engine: Option<Arc<Engine>>,
    /// Remote candidates that arrived before we could use them.
    early: Vec<RTCIceCandidateInit>,
    remote_set: bool,
    tasks: Vec<JoinHandle<()>>,
}

struct Inner {
    client: Client,
    ui: async_channel::Sender<CallEvent>,
    audio: AudioFactory,
    state: Mutex<Option<Call>>,
}

#[derive(Clone)]
pub struct Calls {
    inner: Arc<Inner>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn random_id() -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn base(call_id: &str, party: &str) -> serde_json::Map<String, Value> {
    let Value::Object(m) = json!({"call_id": call_id, "party_id": party, "version": "1"}) else {
        unreachable!()
    };
    m
}

fn metadata() -> Value {
    json!({"msm-stream": {"purpose": "m.usermedia", "audio_muted": false, "video_muted": true}})
}

async fn send(room: &Room, kind: &str, mut content: serde_json::Map<String, Value>, extra: Value) {
    if let Value::Object(e) = extra {
        content.extend(e);
    }
    if let Err(e) = room.send_raw(kind, Value::Object(content)).await {
        tracing::warn!("could not send {kind}: {e}");
    }
}

async fn ice_servers(client: &Client) -> Vec<IceServer> {
    use matrix_sdk::ruma::api::client::voip::get_turn_server_info::v3::Request;
    match client.send(Request::new()).await {
        Ok(r) if !r.uris.is_empty() => vec![IceServer {
            urls: r.uris,
            username: r.username,
            credential: r.password,
        }],
        _ => Vec::new(),
    }
}

impl Calls {
    /// Start listening for calls on `client`; UI updates go to `ui`.
    pub fn start(
        client: &Client,
        ui: async_channel::Sender<CallEvent>,
        audio: AudioFactory,
    ) -> Self {
        let calls = Calls {
            inner: Arc::new(Inner {
                client: client.clone(),
                ui,
                audio,
                state: Mutex::new(None),
            }),
        };
        let c = calls.clone();
        client.add_event_handler(move |ev: OriginalSyncCallInviteEvent, room: Room| {
            let c = c.clone();
            async move {
                let age = now_ms().saturating_sub(u64::from(ev.origin_server_ts.0));
                if age > MAX_AGE_MS {
                    return;
                }
                c.on_invite(
                    room,
                    ev.sender.to_string(),
                    ev.content.call_id.to_string(),
                    ev.content.offer.sdp.clone(),
                    ev.content.invitee.as_ref().map(|u| u.to_string()),
                )
                .await;
            }
        });
        let c = calls.clone();
        client.add_event_handler(move |ev: OriginalSyncCallAnswerEvent| {
            let c = c.clone();
            async move {
                c.on_answer(
                    ev.sender.as_ref(),
                    ev.content.call_id.as_str(),
                    ev.content
                        .party_id
                        .as_ref()
                        .map(|p| p.to_string())
                        .unwrap_or_default(),
                    ev.content.answer.sdp.clone(),
                )
                .await;
            }
        });
        let c = calls.clone();
        client.add_event_handler(move |ev: OriginalSyncCallCandidatesEvent| {
            let c = c.clone();
            async move {
                if Some(ev.sender.to_string()) == c.me() {
                    return;
                }
                let list = ev
                    .content
                    .candidates
                    .iter()
                    .filter(|x| !x.candidate.is_empty())
                    .map(|x| RTCIceCandidateInit {
                        candidate: x.candidate.clone(),
                        sdp_mid: x.sdp_mid.clone(),
                        sdp_mline_index: x
                            .sdp_m_line_index
                            .map(|i| u64::from(i) as u16)
                            .or(Some(0)),
                        ..Default::default()
                    })
                    .collect();
                c.on_candidates(ev.content.call_id.as_str(), list).await;
            }
        });
        let c = calls.clone();
        client.add_event_handler(move |ev: OriginalSyncCallHangupEvent| {
            let c = c.clone();
            async move {
                c.on_remote_end(
                    ev.sender.as_ref(),
                    ev.content.call_id.as_str(),
                    "The other person hung up",
                )
                .await;
            }
        });
        let c = calls.clone();
        client.add_event_handler(move |ev: OriginalSyncCallRejectEvent| {
            let c = c.clone();
            async move {
                c.on_remote_end(
                    ev.sender.as_ref(),
                    ev.content.call_id.as_str(),
                    "Call declined",
                )
                .await;
            }
        });
        // Other clients tell the devices that lost the race to stop ringing.
        let c = calls.clone();
        client.add_event_handler(move |ev: OriginalSyncCallSelectAnswerEvent| {
            let c = c.clone();
            async move {
                let selected = ev.content.selected_party_id.to_string();
                c.on_select_answer(ev.content.call_id.as_str(), &selected)
                    .await;
            }
        });
        calls
    }

    fn me(&self) -> Option<String> {
        self.inner.client.user_id().map(|u| u.to_string())
    }

    fn tell(&self, e: CallEvent) {
        let _ = self.inner.ui.try_send(e);
    }

    /// Whether a call is in progress or ringing.
    pub async fn busy(&self) -> bool {
        self.inner.state.lock().await.is_some()
    }

    /// Call the other person in `room` (a direct chat).
    pub async fn place(&self, room: Room, peer: &str) -> Result<(), String> {
        let mut st = self.inner.state.lock().await;
        if st.is_some() {
            return Err("You are already in a call.".into());
        }
        let call_id = random_id();
        let my_party = random_id();
        let audio = (self.inner.audio)()?;
        let (engine, events) = Engine::new(ice_servers(&self.inner.client).await, audio)
            .await
            .map_err(|e| e.to_string())?;
        let engine = Arc::new(engine);
        let offer = engine.create_offer().await.map_err(|e| e.to_string())?;
        let mut call = Call {
            id: call_id.clone(),
            room: room.clone(),
            my_party: my_party.clone(),
            peer: peer.to_owned(),
            incoming: false,
            answered: false,
            offer_sdp: None,
            engine: Some(engine),
            early: Vec::new(),
            remote_set: false,
            tasks: Vec::new(),
        };
        call.tasks.push(self.pump(events, &call));
        call.tasks.push(self.ring_timeout(&call));
        *st = Some(call);
        drop(st);
        send(
            &room,
            "m.call.invite",
            base(&call_id, &my_party),
            json!({
                "lifetime": RING_MS,
                "offer": {"type": "offer", "sdp": offer},
                "invitee": peer,
                "sdp_stream_metadata": metadata(),
            }),
        )
        .await;
        self.tell(CallEvent::Ringing {
            call_id,
            room_id: room.room_id().to_string(),
        });
        Ok(())
    }

    /// Forward the engine's events: candidates to the other side, state to the UI.
    fn pump(
        &self,
        mut events: tokio::sync::mpsc::UnboundedReceiver<EngineEvent>,
        call: &Call,
    ) -> JoinHandle<()> {
        let this = self.clone();
        let (room, id, party) = (call.room.clone(), call.id.clone(), call.my_party.clone());
        tokio::spawn(async move {
            let mut batch: Vec<Value> = Vec::new();
            loop {
                let ev = if batch.is_empty() {
                    events.recv().await
                } else {
                    // Gather a few candidates into one event.
                    match tokio::time::timeout(Duration::from_millis(150), events.recv()).await {
                        Ok(e) => e,
                        Err(_) => {
                            let c = std::mem::take(&mut batch);
                            send(
                                &room,
                                "m.call.candidates",
                                base(&id, &party),
                                json!({"candidates": c}),
                            )
                            .await;
                            continue;
                        }
                    }
                };
                match ev {
                    Some(EngineEvent::Candidate(c)) => batch.push(json!({
                        "candidate": c.candidate,
                        "sdpMid": c.sdp_mid.unwrap_or_default(),
                        "sdpMLineIndex": c.sdp_mline_index.unwrap_or(0),
                    })),
                    Some(EngineEvent::Connected) => this.tell(CallEvent::Active),
                    Some(EngineEvent::Failed) => {
                        this.finish(&id, Some("ice_failed"), "The connection failed")
                            .await;
                        return;
                    }
                    Some(EngineEvent::Closed) => {}
                    None => return,
                }
            }
        })
    }

    fn ring_timeout(&self, call: &Call) -> JoinHandle<()> {
        let this = self.clone();
        let id = call.id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(RING_MS)).await;
            let unanswered = this
                .inner
                .state
                .lock()
                .await
                .as_ref()
                .is_some_and(|c| c.id == id && !c.answered);
            if unanswered {
                this.finish(&id, Some("invite_timeout"), "No answer").await;
            }
        })
    }

    /// End call `id`: optionally tell the room why, stop media and update the UI.
    async fn finish(&self, id: &str, send_hangup: Option<&str>, message: &str) {
        let call = {
            let mut st = self.inner.state.lock().await;
            if st.as_ref().is_some_and(|c| c.id == id) {
                st.take()
            } else {
                None
            }
        };
        let Some(call) = call else { return };
        for t in &call.tasks {
            t.abort();
        }
        if let Some(e) = &call.engine {
            e.close().await;
        }
        match send_hangup {
            Some("reject") => {
                send(
                    &call.room,
                    "m.call.reject",
                    base(&call.id, &call.my_party),
                    json!({}),
                )
                .await
            }
            Some(reason) => {
                send(
                    &call.room,
                    "m.call.hangup",
                    base(&call.id, &call.my_party),
                    json!({"reason": reason}),
                )
                .await
            }
            None => {}
        }
        self.tell(CallEvent::Ended {
            reason: message.to_owned(),
        });
    }

    /// Hang up, cancel, or decline, whichever fits the current state.
    pub async fn hang_up(&self) {
        let (id, how, msg) = {
            let st = self.inner.state.lock().await;
            let Some(c) = st.as_ref() else { return };
            if c.incoming && !c.answered {
                (c.id.clone(), "reject", "Call declined")
            } else {
                (c.id.clone(), "user_hangup", "Call ended")
            }
        };
        self.finish(&id, Some(how), msg).await;
    }

    pub async fn set_muted(&self, muted: bool) {
        if let Some(e) = self
            .inner
            .state
            .lock()
            .await
            .as_ref()
            .and_then(|c| c.engine.clone())
        {
            e.set_muted(muted);
        }
    }

    async fn on_invite(
        &self,
        room: Room,
        sender: String,
        call_id: String,
        sdp: String,
        invitee: Option<String>,
    ) {
        let me = self.me();
        if me.as_deref() == Some(sender.as_str()) {
            return; // our own invitation, echoed back by sync
        }
        if invitee.is_some() && invitee != me {
            return;
        }
        let mut st = self.inner.state.lock().await;
        if st.is_some() {
            drop(st);
            // Busy: tell them, without disturbing the call in progress.
            send(
                &room,
                "m.call.hangup",
                base(&call_id, &random_id()),
                json!({"reason": "user_busy"}),
            )
            .await;
            return;
        }
        let room_id = room.room_id().to_string();
        *st = Some(Call {
            id: call_id.clone(),
            room,
            my_party: random_id(),
            peer: sender.clone(),
            incoming: true,
            answered: false,
            offer_sdp: Some(sdp),
            engine: None,
            early: Vec::new(),
            remote_set: false,
            tasks: Vec::new(),
        });
        let this = self.clone();
        let id = call_id.clone();
        let t = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(RING_MS)).await;
            let missed = this
                .inner
                .state
                .lock()
                .await
                .as_ref()
                .is_some_and(|c| c.id == id && !c.answered);
            if missed {
                this.finish(&id, None, "Missed call").await;
            }
        });
        if let Some(c) = st.as_mut() {
            c.tasks.push(t);
        }
        drop(st);
        self.tell(CallEvent::Incoming {
            call_id,
            room_id,
            caller: sender,
        });
    }

    /// Accept the incoming call.
    pub async fn answer(&self) -> Result<(), String> {
        let (room, id, party, sdp) = {
            let st = self.inner.state.lock().await;
            let Some(c) = st.as_ref().filter(|c| c.incoming && !c.answered) else {
                return Err("There is no call to answer.".into());
            };
            (
                c.room.clone(),
                c.id.clone(),
                c.my_party.clone(),
                c.offer_sdp.clone().unwrap_or_default(),
            )
        };
        let setup = async {
            let audio = (self.inner.audio)()?;
            let (engine, events) = Engine::new(ice_servers(&self.inner.client).await, audio)
                .await
                .map_err(|e| e.to_string())?;
            let answer = engine.accept_offer(&sdp).await.map_err(|e| e.to_string())?;
            Ok::<_, String>((Arc::new(engine), events, answer))
        };
        let (engine, events, answer) = match setup.await {
            Ok(x) => x,
            Err(e) => {
                self.finish(&id, Some("user_media_failed"), &e).await;
                return Err(e);
            }
        };
        let early = {
            let mut st = self.inner.state.lock().await;
            let Some(c) = st.as_mut().filter(|c| c.id == id) else {
                engine.close().await;
                return Err("The call ended.".into());
            };
            c.answered = true;
            c.engine = Some(engine.clone());
            c.remote_set = true;
            let pump = self.pump(events, c);
            c.tasks.push(pump);
            std::mem::take(&mut c.early)
        };
        send(
            &room,
            "m.call.answer",
            base(&id, &party),
            json!({"answer": {"type": "answer", "sdp": answer}, "sdp_stream_metadata": metadata()}),
        )
        .await;
        for c in early {
            let _ = engine.add_candidate(c).await;
        }
        self.tell(CallEvent::Connecting);
        Ok(())
    }

    async fn on_answer(&self, sender: &str, call_id: &str, party: String, sdp: String) {
        let mine = self.me().as_deref() == Some(sender);
        let (engine, room, my_party, early) = {
            let mut st = self.inner.state.lock().await;
            let Some(c) = st.as_mut().filter(|c| c.id == call_id) else {
                return;
            };
            if c.incoming {
                // We have the call on another device of ours that picked it up.
                let elsewhere = mine && !c.answered;
                drop(st);
                if elsewhere {
                    self.finish(call_id, None, "Answered on another device")
                        .await;
                }
                return;
            }
            if mine || c.answered {
                return;
            }
            c.answered = true;
            c.remote_set = true;
            (
                c.engine.clone(),
                c.room.clone(),
                c.my_party.clone(),
                std::mem::take(&mut c.early),
            )
        };
        let Some(engine) = engine else { return };
        if let Err(e) = engine.set_answer(&sdp).await {
            self.finish(call_id, Some("unknown_error"), &e.to_string())
                .await;
            return;
        }
        self.tell(CallEvent::Connecting);
        send(
            &room,
            "m.call.select_answer",
            base(call_id, &my_party),
            json!({"selected_party_id": party}),
        )
        .await;
        for c in early {
            let _ = engine.add_candidate(c).await;
        }
    }

    async fn on_candidates(&self, call_id: &str, list: Vec<RTCIceCandidateInit>) {
        let engine = {
            let mut st = self.inner.state.lock().await;
            let Some(c) = st.as_mut().filter(|c| c.id == call_id) else {
                return;
            };
            if !c.remote_set {
                c.early.extend(list);
                return;
            }
            c.engine.clone()
        };
        if let Some(e) = engine {
            for c in list {
                let _ = e.add_candidate(c).await;
            }
        }
    }

    async fn on_remote_end(&self, sender: &str, call_id: &str, message: &str) {
        let mine = self.me().as_deref() == Some(sender);
        let relevant = {
            let st = self.inner.state.lock().await;
            st.as_ref().is_some_and(|c| {
                c.id == call_id && (c.peer == sender || (mine && c.incoming && !c.answered))
            })
        };
        if relevant {
            let msg = if mine {
                "Handled on another device"
            } else {
                message
            };
            self.finish(call_id, None, msg).await;
        }
    }

    async fn on_select_answer(&self, call_id: &str, selected: &str) {
        let lost = {
            let st = self.inner.state.lock().await;
            st.as_ref().is_some_and(|c| {
                c.id == call_id && c.incoming && c.answered && c.my_party != selected
            })
        };
        if lost {
            self.finish(call_id, None, "The other person picked another device")
                .await;
        }
    }
}
