//! System tray icon via the StatusNotifierItem D-Bus protocol.
//!
//! GTK3's `StatusIcon` is X11-only and does not work on Wayland, so we speak
//! StatusNotifierItem directly (supported by KDE, MATE, XFCE, Cinnamon and
//! GNOME with the AppIndicator extension). Menu callbacks run on the tray's
//! own thread, so they only send commands over a channel to the GTK main thread.

use ksni::menu::{RadioGroup, RadioItem, StandardItem};
use ksni::{Category, Icon, MenuItem, ToolTip};
use msm::icon::status_icon_argb;
use msm::matrix::presence::UiStatus;

const ICON_SIZE: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    SetStatus(UiStatus),
    ToggleWindow,
    Quit,
}

#[derive(Debug)]
pub struct MsmTray {
    pub status: UiStatus,
    pub window_visible: bool,
    pub tx: async_channel::Sender<TrayCommand>,
}

impl ksni::Tray for MsmTray {
    fn id(&self) -> String {
        "com.matrixmessenger.Msm".into()
    }

    fn title(&self) -> String {
        "Matrix Messenger".into()
    }

    fn category(&self) -> Category {
        Category::Communications
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![Icon {
            width: ICON_SIZE as i32,
            height: ICON_SIZE as i32,
            data: status_icon_argb(self.status, ICON_SIZE),
        }]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: format!("Matrix Messenger - {}", self.status.label()),
            ..Default::default()
        }
    }

    // Left click toggles the main window.
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.try_send(TrayCommand::ToggleWindow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            RadioGroup {
                selected: self.status.index(),
                select: Box::new(|this: &mut Self, i| {
                    let s = UiStatus::ALL[i.min(UiStatus::ALL.len() - 1)];
                    this.status = s;
                    let _ = this.tx.try_send(TrayCommand::SetStatus(s));
                }),
                options: UiStatus::ALL
                    .iter()
                    .map(|s| RadioItem {
                        label: s.label().into(),
                        ..Default::default()
                    })
                    .collect(),
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: if self.window_visible {
                    "Hide window"
                } else {
                    "Show window"
                }
                .into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.try_send(TrayCommand::ToggleWindow);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.try_send(TrayCommand::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
