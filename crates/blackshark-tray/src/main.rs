mod proxy;

use std::sync::{Arc, Mutex};

use anyhow::Result;
use ksni::{self, menu::*, Icon, Tray};
use zbus::Connection;

use proxy::HeadsetProxy;

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct HeadsetState {
    connected:     bool,
    battery_pct:   u8,
    charging:      bool,
    sidetone:      u8,
    thx_enabled:   bool,
    anc_enabled:   bool,
    anc_level:     u8,
    power_savings: u8,
}

// ---------------------------------------------------------------------------
// Tray
// ---------------------------------------------------------------------------

struct BlacksharkTray {
    state:  Arc<Mutex<HeadsetState>>,
    conn:   Connection,
    rt:     tokio::runtime::Handle,
}

impl Tray for BlacksharkTray {
    fn id(&self) -> String {
        "blackshark-v3-pro".into()
    }

    fn icon_name(&self) -> String {
        "audio-headset".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![]
    }

    fn title(&self) -> String {
        "BlackShark V3 Pro".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let s = self.state.lock().unwrap();
        let description = if !s.connected {
            "Disconnected".into()
        } else {
            let charging = if s.charging { " ⚡" } else { "" };
            format!("{}%{} — Sidetone {}", s.battery_pct, charging, s.sidetone)
        };
        ksni::ToolTip {
            icon_name:   String::new(),
            icon_pixmap: vec![],
            title:       "BlackShark V3 Pro".into(),
            description,
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let s = self.state.lock().unwrap().clone();

        let battery_label = if !s.connected {
            "Not connected".into()
        } else {
            let charging = if s.charging { " ⚡" } else { "" };
            format!("Battery: {}%{}", s.battery_pct, charging)
        };

        let mut items: Vec<MenuItem<Self>> = vec![
            MenuItem::Standard(StandardItem {
                label:   battery_label,
                enabled: false,
                ..Default::default()
            }),
            MenuItem::Separator,
        ];

        if s.connected {
            // Sidetone submenu
            let sidetone = s.sidetone;
            items.push(MenuItem::SubMenu(SubMenu {
                label:   format!("Sidetone: {sidetone}"),
                submenu: (0u8..=15)
                    .map(|lvl| {
                        MenuItem::Standard(StandardItem {
                            label:     format!("{lvl}"),
                            icon_name: if lvl == sidetone { "object-select" } else { "" }.into(),
                            activate:  Box::new(move |tray: &mut Self| {
                                let conn = tray.conn.clone();
                                tray.rt.spawn(async move {
                                    if let Ok(proxy) = HeadsetProxy::new(&conn).await {
                                        let _ = proxy.set_sidetone(lvl).await;
                                    }
                                });
                            }),
                            ..Default::default()
                        })
                    })
                    .collect(),
                ..Default::default()
            }));

            // THX toggle
            let thx = s.thx_enabled;
            items.push(MenuItem::Standard(StandardItem {
                label:    format!("THX Spatial: {}", if thx { "On ✓" } else { "Off" }),
                activate: Box::new(move |tray: &mut Self| {
                    let conn = tray.conn.clone();
                    tray.rt.spawn(async move {
                        if let Ok(proxy) = HeadsetProxy::new(&conn).await {
                            let _ = proxy.set_thx(!thx).await;
                        }
                    });
                }),
                ..Default::default()
            }));

            // ANC toggle
            let anc     = s.anc_enabled;
            let anc_lvl = s.anc_level;
            items.push(MenuItem::Standard(StandardItem {
                label:    format!("ANC: {}", if anc { format!("On ✓ (level {anc_lvl})") } else { "Off".into() }),
                activate: Box::new(move |tray: &mut Self| {
                    let conn = tray.conn.clone();
                    tray.rt.spawn(async move {
                        if let Ok(proxy) = HeadsetProxy::new(&conn).await {
                            let _ = proxy.set_anc(!anc, anc_lvl).await;
                        }
                    });
                }),
                ..Default::default()
            }));

            // Power savings submenu
            let ps = s.power_savings;
            items.push(MenuItem::SubMenu(SubMenu {
                label:   format!("Power savings: {}", if ps == 0 { "Off".into() } else { format!("{ps} min") }),
                submenu: [0u8, 15, 30, 45, 60]
                    .iter()
                    .map(|&m| {
                        let label = if m == 0 { "Off".into() } else { format!("{m} min") };
                        MenuItem::Standard(StandardItem {
                            label,
                            icon_name: if m == ps { "object-select" } else { "" }.into(),
                            activate:  Box::new(move |tray: &mut Self| {
                                let conn = tray.conn.clone();
                                tray.rt.spawn(async move {
                                    if let Ok(proxy) = HeadsetProxy::new(&conn).await {
                                        let _ = proxy.set_power_savings(m).await;
                                    }
                                });
                            }),
                            ..Default::default()
                        })
                    })
                    .collect(),
                ..Default::default()
            }));

            items.push(MenuItem::Separator);
        }

        items.push(MenuItem::Standard(StandardItem {
            label:    "Quit".into(),
            activate: Box::new(|_| std::process::exit(0)),
            ..Default::default()
        }));

        items
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let conn  = Connection::session().await?;
    let state = Arc::new(Mutex::new(HeadsetState::default()));

    // Load initial state from daemon
    if let Ok(proxy) = HeadsetProxy::new(&conn).await {
        if let Ok(connected) = proxy.connected().await {
            let mut s = state.lock().unwrap();
            s.connected = connected;
            if connected {
                s.battery_pct   = proxy.battery_percentage().await.unwrap_or(0);
                s.sidetone      = proxy.sidetone().await.unwrap_or(0);
                s.thx_enabled   = proxy.thx_enabled().await.unwrap_or(false);
                s.anc_enabled   = proxy.anc_enabled().await.unwrap_or(false);
                s.anc_level     = proxy.anc_level().await.unwrap_or(1);
                s.power_savings = proxy.power_savings_minutes().await.unwrap_or(0);
            }
        }
    }

    // Build tray service and get handle before spawning
    let service = ksni::TrayService::new(BlacksharkTray {
        state: state.clone(),
        conn:  conn.clone(),
        rt:    tokio::runtime::Handle::current(),
    });
    let handle = service.handle();
    service.spawn();

    // Watch D-Bus signals and update tray state
    let state2 = state.clone();
    tokio::spawn(async move {
        use futures_util::StreamExt;
        let Ok(proxy) = HeadsetProxy::new(&conn).await else { return };

        let mut battery_stream   = proxy.receive_battery_changed().await.ok();
        let mut connected_stream = proxy.receive_connected_changed().await;
        let mut sidetone_stream  = proxy.receive_sidetone_changed().await;
        let mut thx_stream       = proxy.receive_thx_enabled_changed().await;
        let mut anc_stream       = proxy.receive_anc_enabled_changed().await;

        loop {
            tokio::select! {
                Some(sig) = async { battery_stream.as_mut()?.next().await } => {
                    if let Ok(args) = sig.args() {
                        let mut s = state2.lock().unwrap();
                        s.battery_pct = args.percentage;
                        s.charging    = args.charging;
                    }
                    handle.update(|_| {});
                }
                Some(change) = connected_stream.next() => {
                    if let Ok(val) = change.get().await {
                        state2.lock().unwrap().connected = val;
                    }
                    handle.update(|_| {});
                }
                Some(change) = sidetone_stream.next() => {
                    if let Ok(val) = change.get().await {
                        state2.lock().unwrap().sidetone = val;
                    }
                    handle.update(|_| {});
                }
                Some(change) = thx_stream.next() => {
                    if let Ok(val) = change.get().await {
                        state2.lock().unwrap().thx_enabled = val;
                    }
                    handle.update(|_| {});
                }
                Some(change) = anc_stream.next() => {
                    if let Ok(val) = change.get().await {
                        state2.lock().unwrap().anc_enabled = val;
                    }
                    handle.update(|_| {});
                }
            }
        }
    });

    std::future::pending::<()>().await;
    Ok(())
}
