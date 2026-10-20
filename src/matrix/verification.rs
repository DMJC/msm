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

//! Interactive verification (emoji comparison or QR scanning) of other users and
//! of our own devices.
//!
//! [`run`] drives one verification request and reports what the UI should show;
//! the UI answers with [`VerifyCmd`]. Only a person can confirm a match (emoji
//! comparison, or that the other side really scanned our QR code): nothing here
//! confirms on their behalf.

use futures_util::StreamExt;
use matrix_sdk::Client;
use matrix_sdk::encryption::verification::SasState;
use matrix_sdk::encryption::verification::{
    QrVerification, QrVerificationData, QrVerificationState, SasVerification, Verification,
    VerificationRequest, VerificationRequestState,
};
use matrix_sdk::ruma::UserId;
use matrix_sdk::ruma::events::key::verification::VerificationMethod;
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
    /// Both sides agreed to verify: offer our QR code (if one can be made, as a
    /// grid of dark modules) to be scanned, scanning theirs, or comparing emoji.
    Ready {
        qr: Option<Vec<Vec<bool>>>,
    },
    /// Starting the emoji exchange.
    Starting,
    /// They scanned our QR code. Confirm only if they really did.
    QrScanned,
    /// Compare these (symbol, name) pairs with the other side.
    Emojis(Vec<(String, String)>),
    /// We confirmed; waiting for the other side.
    Confirmed,
    Done,
    Cancelled(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyCmd {
    Accept,
    /// Compare emoji instead of scanning.
    StartEmoji,
    /// The raw payload of a QR code read from their screen.
    ScanQr(Vec<u8>),
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

/// The SDK's default does not offer scanning, so say what we can do: show and
/// scan QR codes (the camera is optional at the UI) and compare emoji.
fn methods() -> Vec<VerificationMethod> {
    vec![
        VerificationMethod::SasV1,
        VerificationMethod::QrCodeShowV1,
        VerificationMethod::QrCodeScanV1,
        VerificationMethod::ReciprocateV1,
    ]
}

/// Dark modules of `qr`, row by row, with no quiet zone.
fn qr_grid(qr: &QrVerification) -> Option<Vec<Vec<bool>>> {
    let code = qr.to_qr_code().ok()?;
    let w = code.width();
    let colors = code.to_colors();
    Some(
        colors
            .chunks(w)
            .map(|row| row.iter().map(|c| *c == qrcode::Color::Dark).collect())
            .collect(),
    )
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
        .request_verification_with_methods(methods())
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

/// A request from one of our own devices is only actionable if we already know that
/// device, which a client that was offline when it signed in may not.
pub async fn refresh_own_devices(client: &Client) {
    if let Some(me) = client.user_id() {
        let _ = client.encryption().request_user_identity(me).await;
    }
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
    let qr_slot: std::sync::Arc<tokio::sync::Mutex<Option<QrVerification>>> = Default::default();
    let ui_tx_cmd = ui_tx.clone();

    // Commands from the UI.
    let (req, slot, qslot) = (request.clone(), sas_slot.clone(), qr_slot.clone());
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            let sas = slot.lock().await.clone();
            let qr = qslot.lock().await.clone();
            let _ = match (cmd, sas, qr) {
                (VerifyCmd::Accept, ..) => req.accept_with_methods(methods()).await,
                (VerifyCmd::StartEmoji, ..) => req.start_sas().await.map(|_| ()),
                (VerifyCmd::ScanQr(bytes), ..) => {
                    match QrVerificationData::from_bytes(bytes) {
                        Ok(data) => match req.scan_qr_code(data).await {
                            Ok(Some(qr)) => {
                                *qslot.lock().await = Some(qr.clone());
                                tokio::spawn(follow_qr(qr, ui_tx_cmd.clone()));
                            }
                            Ok(None) => {
                                let _ = ui_tx_cmd.send(VerifyUi::Cancelled(
                                    "that QR code cannot be used for this verification".into(),
                                ));
                            }
                            Err(e) => {
                                let _ = ui_tx_cmd.send(VerifyUi::Cancelled(e.to_string()));
                            }
                        },
                        Err(_) => {
                            let _ = ui_tx_cmd
                                .send(VerifyUi::Cancelled("that is not a Matrix QR code".into()));
                        }
                    }
                    Ok(())
                }
                // "They scanned me" takes precedence: it is the only time a QR waits on us.
                (VerifyCmd::Match, _, Some(q)) if q.has_been_scanned() => q.confirm().await,
                (VerifyCmd::Match, Some(s), _) => s.confirm().await,
                (VerifyCmd::Mismatch, _, Some(q)) if q.has_been_scanned() => q.cancel().await,
                (VerifyCmd::Mismatch, Some(s), _) => s.mismatch().await,
                (VerifyCmd::Cancel, _, Some(q)) => q.cancel().await,
                (VerifyCmd::Cancel, Some(s), _) => s.cancel().await,
                (VerifyCmd::Cancel, None, None) => req.cancel().await,
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
                    // A second QR object for the same flow makes the SDK cancel both, so
                    // generate once and reuse it if this state is reported again.
                    let mut slot = qr_slot.lock().await;
                    if slot.is_none()
                        && let Ok(Some(qr)) = request.generate_qr_code().await
                    {
                        tokio::spawn(follow_qr(qr.clone(), ui_tx.clone()));
                        *slot = Some(qr);
                    }
                    let grid = slot.as_ref().and_then(qr_grid);
                    drop(slot);
                    let _ = ui_tx.send(VerifyUi::Ready { qr: grid });
                }
                VerificationRequestState::Transitioned { verification } => match verification {
                    Verification::SasV1(sas) => {
                        let _ = ui_tx.send(VerifyUi::Starting);
                        *sas_slot.lock().await = Some(sas.clone());
                        tokio::spawn(follow_sas(sas, ui_tx.clone()));
                    }
                    Verification::QrV1(qr) => {
                        let mut slot = qr_slot.lock().await;
                        if slot.is_none() {
                            *slot = Some(qr.clone());
                            tokio::spawn(follow_qr(qr, ui_tx.clone()));
                        }
                    }
                    _ => {}
                },
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
    // The side that received the SAS start must accept it. `we_started` follows the
    // request, not the SAS, so when the accepting device picks emoji it is wrong on
    // both sides; accepting is a no-op for the side that sent the start.
    let _ = sas.accept().await;
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

async fn follow_qr(qr: QrVerification, ui: mpsc::UnboundedSender<VerifyUi>) {
    let mut changes = qr.changes();
    while let Some(state) = changes.next().await {
        match state {
            QrVerificationState::Scanned => {
                let _ = ui.send(VerifyUi::QrScanned);
            }
            // We scanned theirs (Reciprocated) or confirmed their scan: now they decide.
            QrVerificationState::Reciprocated | QrVerificationState::Confirmed => {
                let _ = ui.send(VerifyUi::Confirmed);
            }
            // The request stream reports Done/Cancelled for the whole flow.
            QrVerificationState::Done { .. } | QrVerificationState::Cancelled(_) => return,
            _ => {}
        }
    }
}
