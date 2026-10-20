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

//! Interactive emoji (SAS) verification of other users and of our own devices.
//!
//! [`run`] drives one verification request and reports what the UI should show;
//! the UI answers with [`VerifyCmd`]. Only the person comparing the emoji can
//! confirm a match: nothing here confirms on their behalf.

use futures_util::StreamExt;
use matrix_sdk::Client;
use matrix_sdk::encryption::verification::SasState;
use matrix_sdk::encryption::verification::{
    SasVerification, Verification, VerificationRequest, VerificationRequestState,
};
use matrix_sdk::ruma::UserId;
use matrix_sdk::ruma::events::key::verification::request::ToDeviceKeyVerificationRequestEvent;
use matrix_sdk::ruma::events::room::message::{MessageType, OriginalSyncRoomMessageEvent};
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("{0} has no encryption keys yet (they need to set up cross-signing first)")]
    NoIdentity(String),
    #[error("not logged in")]
    NotLoggedIn,
    #[error("{0}")]
    Matrix(String),
}

fn mx(e: impl std::fmt::Display) -> VerifyError {
    VerifyError::Matrix(e.to_string())
}

/// What the verification dialog should show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyUi {
    /// We asked; waiting for the other side to accept.
    Waiting,
    /// They asked; waiting for us to accept or decline.
    Requested,
    /// Starting the emoji exchange.
    Starting,
    /// Compare these (symbol, name) pairs with the other side.
    Emojis(Vec<(String, String)>),
    /// We confirmed; waiting for the other side.
    Confirmed,
    Done,
    Cancelled(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyCmd {
    Accept,
    Match,
    Mismatch,
    Cancel,
}

pub struct Flow {
    /// Who is being verified (their Matrix ID; ours for self-verification).
    pub other_user: String,
    pub self_verification: bool,
    pub ui: mpsc::UnboundedReceiver<VerifyUi>,
    pub cmd: mpsc::UnboundedSender<VerifyCmd>,
}

/// Ask `mxid` (or ourselves, to verify another of our devices) to verify.
pub async fn request(client: &Client, mxid: &str) -> Result<VerificationRequest, VerifyError> {
    let id = UserId::parse(mxid).map_err(mx)?;
    client
        .encryption()
        .request_user_identity(&id)
        .await
        .map_err(mx)?
        .ok_or_else(|| VerifyError::NoIdentity(mxid.to_owned()))?
        .request_verification()
        .await
        .map_err(mx)
}

/// Request verification of our own other devices.
pub async fn request_self(client: &Client) -> Result<VerificationRequest, VerifyError> {
    let me = client
        .user_id()
        .ok_or(VerifyError::NotLoggedIn)?
        .to_string();
    request(client, &me).await
}

/// The SDK registers a request while processing the same sync that fires our
/// event handlers, so it may not be visible for a moment.
async fn find_request(
    client: &Client,
    sender: &UserId,
    flow_id: &str,
) -> Option<VerificationRequest> {
    for _ in 0..20 {
        if let Some(r) = client
            .encryption()
            .get_verification_request(sender, flow_id)
            .await
        {
            return Some(r);
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    None
}

/// Forward incoming verification requests (to-device and in-room) to `tx`.
pub fn watch_incoming(client: &Client, tx: async_channel::Sender<VerificationRequest>) {
    let t = tx.clone();
    client.add_event_handler(
        move |ev: ToDeviceKeyVerificationRequestEvent, client: Client| {
            let t = t.clone();
            async move {
                if let Some(r) =
                    find_request(&client, &ev.sender, ev.content.transaction_id.as_str()).await
                {
                    let _ = t.send(r).await;
                }
            }
        },
    );
    client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, client: Client| {
        let t = tx.clone();
        async move {
            if matches!(ev.content.msgtype, MessageType::VerificationRequest(_))
                && let Some(r) = find_request(&client, &ev.sender, ev.event_id.as_str()).await
            {
                let _ = t.send(r).await;
            }
        }
    });
}

/// Drive `request` until it finishes. Must be called inside the Tokio runtime.
pub fn run(request: VerificationRequest) -> Flow {
    let (ui_tx, ui_rx) = mpsc::unbounded_channel();
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<VerifyCmd>();
    let flow = Flow {
        other_user: request.other_user_id().to_string(),
        self_verification: request.is_self_verification(),
        ui: ui_rx,
        cmd: cmd_tx,
    };
    let sas_slot: std::sync::Arc<tokio::sync::Mutex<Option<SasVerification>>> = Default::default();

    // Commands from the UI.
    let (req, slot) = (request.clone(), sas_slot.clone());
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            let sas = slot.lock().await.clone();
            let _ = match (cmd, sas) {
                (VerifyCmd::Accept, _) => req.accept().await,
                (VerifyCmd::Match, Some(s)) => s.confirm().await,
                (VerifyCmd::Mismatch, Some(s)) => s.mismatch().await,
                (VerifyCmd::Cancel, Some(s)) => s.cancel().await,
                (VerifyCmd::Cancel, None) => req.cancel().await,
                _ => Ok(()),
            };
        }
    });

    // Progress of the request itself.
    tokio::spawn(async move {
        let mut changes = request.changes();
        let mut state = request.state();
        loop {
            match state {
                VerificationRequestState::Created { .. } => {
                    let _ = ui_tx.send(VerifyUi::Waiting);
                }
                VerificationRequestState::Requested { .. } => {
                    let _ = ui_tx.send(VerifyUi::Requested);
                }
                VerificationRequestState::Ready { .. } => {
                    let _ = ui_tx.send(VerifyUi::Starting);
                    if request.we_started() {
                        let _ = request.start_sas().await;
                    }
                }
                VerificationRequestState::Transitioned { verification } => {
                    if let Verification::SasV1(sas) = verification {
                        *sas_slot.lock().await = Some(sas.clone());
                        tokio::spawn(follow_sas(sas, ui_tx.clone()));
                    }
                }
                VerificationRequestState::Done => {
                    let _ = ui_tx.send(VerifyUi::Done);
                    return;
                }
                VerificationRequestState::Cancelled(info) => {
                    let _ = ui_tx.send(VerifyUi::Cancelled(info.reason().to_owned()));
                    return;
                }
            }
            match changes.next().await {
                Some(next) => state = next,
                None => return,
            }
        }
    });
    flow
}

async fn follow_sas(sas: SasVerification, ui: mpsc::UnboundedSender<VerifyUi>) {
    // The side that did not start the flow must accept it.
    if !sas.we_started() {
        let _ = sas.accept().await;
    }
    let mut changes = sas.changes();
    while let Some(state) = changes.next().await {
        match state {
            SasState::KeysExchanged {
                emojis: Some(e), ..
            } => {
                let list = e
                    .emojis
                    .iter()
                    .map(|e| (e.symbol.to_owned(), e.description.to_owned()))
                    .collect();
                let _ = ui.send(VerifyUi::Emojis(list));
            }
            SasState::Confirmed => {
                let _ = ui.send(VerifyUi::Confirmed);
            }
            // The request stream reports Done/Cancelled for the whole flow.
            SasState::Done { .. } | SasState::Cancelled(_) => return,
            _ => {}
        }
    }
}
