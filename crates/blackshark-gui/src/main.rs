mod pipewire;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use blackshark_client::HeadsetProxy;
use slint::{ComponentHandle, ModelRc, VecModel};
use zbus::Connection;

slint::include_modules!();

async fn fetch_streams() -> Vec<SinkInputRow> {
    pipewire::list_sink_inputs()
        .await
        .into_iter()
        .map(|s| SinkInputRow {
            id:    s.id as i32,
            name:  s.app_name.into(),
            route: s.route.into(),
        })
        .collect()
}

async fn refresh_routing(
    rules: &Arc<Mutex<HashMap<String, String>>>,
    window_weak: &slint::Weak<MainWindow>,
) {
    let streams = fetch_streams().await;
    let rules_vec: Vec<RouteRule> = rules
        .lock()
        .unwrap()
        .iter()
        .map(|(name, route)| RouteRule {
            app_name: name.as_str().into(),
            route:    route.as_str().into(),
        })
        .collect();
    let w = window_weak.clone();
    slint::invoke_from_event_loop(move || {
        if let Some(win) = w.upgrade() {
            win.set_streams(ModelRc::new(VecModel::from(streams)));
            win.set_rules(ModelRc::new(VecModel::from(rules_vec)));
        }
    })
    .ok();
}

async fn systemctl(args: &[&str]) {
    let _ = tokio::process::Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .await;
}

async fn daemon_status() -> String {
    let out = tokio::process::Command::new("systemctl")
        .args(["--user", "is-active", "blacksharkd"])
        .output()
        .await;
    match out {
        Ok(o) => String::from_utf8(o.stdout).unwrap_or_default().trim().to_owned(),
        Err(_) => "unknown".to_owned(),
    }
}

/// Create null sinks and loopbacks; returns the list of module IDs created.
async fn setup_sinks(mix: u8) -> Vec<u32> {
    let mut modules = Vec::new();
    pipewire::cleanup_stale_sinks().await;
    let Some(headset_sink) = pipewire::find_headset_sink().await else {
        eprintln!("blackshark-gui: could not find headset sink — game/chat mix unavailable");
        return modules;
    };
    for (name, desc) in [
        ("blackshark-game", "BlackShark-Game"),
        ("blackshark-chat", "BlackShark-Chat"),
    ] {
        match pipewire::load_null_sink(name, desc).await {
            Ok(id) => modules.push(id),
            Err(e) => {
                eprintln!("blackshark-gui: failed to create {name} sink: {e}");
                return modules;
            }
        }
        let monitor = format!("{name}.monitor");
        match pipewire::load_loopback(&monitor, &headset_sink).await {
            Ok(id) => modules.push(id),
            Err(e) => eprintln!("blackshark-gui: failed to create {name} loopback: {e}"),
        }
    }
    pipewire::apply_mix_volumes(mix).await;
    modules
}

#[tokio::main]
async fn main() -> Result<()> {
    let conn = Connection::session().await?;
    let window = MainWindow::new()?;

    // PipeWire state owned by the GUI process.
    let modules: Arc<Mutex<Vec<u32>>>                    = Arc::new(Mutex::new(Vec::new()));
    let rules:   Arc<Mutex<HashMap<String, String>>>     = Arc::new(Mutex::new(HashMap::new()));
    let current_mix: Arc<AtomicU8>                       = Arc::new(AtomicU8::new(50));

    // Load initial state from daemon.
    if let Ok(proxy) = HeadsetProxy::new(&conn).await {
        if let Ok(connected) = proxy.connected().await {
            window.set_connected(connected);
            if connected {
                window.set_battery_pct(proxy.battery_percentage().await.unwrap_or(0) as i32);
                window.set_eq_preset(proxy.eq_preset().await.unwrap_or(0) as i32);
                window.set_game_chat_mix(50);
                window.set_sidetone(proxy.sidetone().await.unwrap_or(0) as i32);
                window.set_thx_enabled(proxy.thx_enabled().await.unwrap_or(false));
                window.set_anc_enabled(proxy.anc_enabled().await.unwrap_or(false));
                window.set_anc_level(proxy.anc_level().await.unwrap_or(1) as i32);
                window.set_power_savings(proxy.power_savings_minutes().await.unwrap_or(0) as i32);

                let new_mods = setup_sinks(50).await;
                let active = !new_mods.is_empty();
                modules.lock().unwrap().extend(new_mods);
                window.set_sinks_active(active);
            }
        }
    }

    // Initial routing view.
    window.set_streams(ModelRc::new(VecModel::from(fetch_streams().await)));
    window.set_rules(ModelRc::new(VecModel::from(Vec::<RouteRule>::new())));

    // Initial daemon status.
    window.set_daemon_status(daemon_status().await.into());

    // Wire up headset-control callbacks (go via D-Bus as before).
    {
        let conn = conn.clone();
        window.on_set_eq(move |preset| {
            let conn = conn.clone();
            tokio::spawn(async move {
                if let Ok(proxy) = HeadsetProxy::new(&conn).await {
                    let _ = proxy.set_eq(preset as u8).await;
                }
            });
        });
    }

    {
        let conn = conn.clone();
        window.on_set_sidetone(move |level| {
            let conn = conn.clone();
            tokio::spawn(async move {
                if let Ok(proxy) = HeadsetProxy::new(&conn).await {
                    let _ = proxy.set_sidetone(level as u8).await;
                }
            });
        });
    }

    {
        let conn = conn.clone();
        window.on_set_thx(move |enabled| {
            let conn = conn.clone();
            tokio::spawn(async move {
                if let Ok(proxy) = HeadsetProxy::new(&conn).await {
                    let _ = proxy.set_thx(enabled).await;
                }
            });
        });
    }

    {
        let conn = conn.clone();
        window.on_set_anc(move |enabled, level| {
            let conn = conn.clone();
            tokio::spawn(async move {
                if let Ok(proxy) = HeadsetProxy::new(&conn).await {
                    let _ = proxy.set_anc(enabled, level as u8).await;
                }
            });
        });
    }

    {
        let conn = conn.clone();
        window.on_set_power_savings(move |minutes| {
            let conn = conn.clone();
            tokio::spawn(async move {
                if let Ok(proxy) = HeadsetProxy::new(&conn).await {
                    let _ = proxy.set_power_savings(minutes as u8).await;
                }
            });
        });
    }

    // Game/chat mix — handled locally, no D-Bus round-trip needed.
    {
        let current_mix = current_mix.clone();
        window.on_set_game_chat_mix(move |mix| {
            let mix = mix as u8;
            current_mix.store(mix, Ordering::Relaxed);
            tokio::spawn(async move {
                pipewire::apply_mix_volumes(mix).await;
            });
        });
    }

    // Routing callbacks — handled locally.
    {
        let rules = rules.clone();
        let window_weak = window.as_weak();
        window.on_set_route(move |id, route| {
            let rules = rules.clone();
            let window_weak = window_weak.clone();
            tokio::spawn(async move {
                let inputs = pipewire::list_sink_inputs().await;
                let Some(input) = inputs.iter().find(|s| s.id == id as u32) else { return };
                let app_name = input.app_name.clone();

                let sink_name = match route.as_str() {
                    "game" => "blackshark-game",
                    "chat" => "blackshark-chat",
                    "" => {
                        rules.lock().unwrap().remove(&app_name);
                        if let Some(headset) = pipewire::find_headset_sink().await {
                            pipewire::move_sink_input(id as u32, &headset).await;
                        }
                        refresh_routing(&rules, &window_weak).await;
                        return;
                    }
                    _ => return,
                };

                rules.lock().unwrap().insert(app_name.clone(), route.to_string());
                for inp in inputs.iter().filter(|s| s.app_name == app_name) {
                    pipewire::move_sink_input(inp.id, sink_name).await;
                }
                refresh_routing(&rules, &window_weak).await;
            });
        });
    }

    {
        let rules = rules.clone();
        let window_weak = window.as_weak();
        window.on_remove_rule(move |name| {
            let rules = rules.clone();
            let window_weak = window_weak.clone();
            tokio::spawn(async move {
                rules.lock().unwrap().remove(name.as_str());
                refresh_routing(&rules, &window_weak).await;
            });
        });
    }

    {
        let rules = rules.clone();
        let window_weak = window.as_weak();
        window.on_refresh_streams(move || {
            let rules = rules.clone();
            let window_weak = window_weak.clone();
            tokio::spawn(async move {
                refresh_routing(&rules, &window_weak).await;
            });
        });
    }

    // Background task: watch D-Bus signals and update UI.
    // Also manages PipeWire sink lifecycle on connect/disconnect.
    {
        let window_weak  = window.as_weak();
        let conn         = conn.clone();
        let modules      = modules.clone();
        let current_mix  = current_mix.clone();
        tokio::spawn(async move {
            use futures_util::StreamExt;
            let Ok(proxy) = HeadsetProxy::new(&conn).await else { return };

            let mut battery_stream   = proxy.receive_battery_changed().await.ok();
            let mut connected_stream = proxy.receive_connected_changed().await;
            let mut eq_stream        = proxy.receive_eq_preset_changed().await;
            let mut sidetone_stream  = proxy.receive_sidetone_changed().await;
            let mut thx_stream       = proxy.receive_thx_enabled_changed().await;
            let mut anc_stream       = proxy.receive_anc_enabled_changed().await;
            let mut anc_level_stream = proxy.receive_anc_level_changed().await;
            let mut ps_stream        = proxy.receive_power_savings_minutes_changed().await;

            loop {
                tokio::select! {
                    Some(sig) = async { battery_stream.as_mut()?.next().await } => {
                        if let Ok(args) = sig.args() {
                            let pct      = args.percentage as i32;
                            let charging = args.charging;
                            let w = window_weak.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(win) = w.upgrade() {
                                    win.set_battery_pct(pct);
                                    win.set_charging(charging);
                                }
                            }).ok();
                        }
                    }
                    Some(change) = connected_stream.next() => {
                        if let Ok(val) = change.get().await {
                            let w = window_weak.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(win) = w.upgrade() { win.set_connected(val); }
                            }).ok();

                            let has_modules = !modules.lock().unwrap().is_empty();
                            match (has_modules, val) {
                                (false, true) => {
                                    let mix = current_mix.load(Ordering::Relaxed);
                                    let new_mods = setup_sinks(mix).await;
                                    let active = !new_mods.is_empty();
                                    modules.lock().unwrap().extend(new_mods);
                                    let w = window_weak.clone();
                                    slint::invoke_from_event_loop(move || {
                                        if let Some(win) = w.upgrade() { win.set_sinks_active(active); }
                                    }).ok();
                                }
                                (true, false) => {
                                    let mods: Vec<u32> = modules.lock().unwrap().drain(..).collect();
                                    for id in mods {
                                        pipewire::unload_module(id).await;
                                    }
                                    let w = window_weak.clone();
                                    slint::invoke_from_event_loop(move || {
                                        if let Some(win) = w.upgrade() { win.set_sinks_active(false); }
                                    }).ok();
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(change) = eq_stream.next() => {
                        if let Ok(val) = change.get().await {
                            let w = window_weak.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(win) = w.upgrade() { win.set_eq_preset(val as i32); }
                            }).ok();
                        }
                    }
                    Some(change) = sidetone_stream.next() => {
                        if let Ok(val) = change.get().await {
                            let w = window_weak.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(win) = w.upgrade() { win.set_sidetone(val as i32); }
                            }).ok();
                        }
                    }
                    Some(change) = thx_stream.next() => {
                        if let Ok(val) = change.get().await {
                            let w = window_weak.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(win) = w.upgrade() { win.set_thx_enabled(val); }
                            }).ok();
                        }
                    }
                    Some(change) = anc_stream.next() => {
                        if let Ok(val) = change.get().await {
                            let w = window_weak.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(win) = w.upgrade() { win.set_anc_enabled(val); }
                            }).ok();
                        }
                    }
                    Some(change) = anc_level_stream.next() => {
                        if let Ok(val) = change.get().await {
                            let w = window_weak.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(win) = w.upgrade() { win.set_anc_level(val as i32); }
                            }).ok();
                        }
                    }
                    Some(change) = ps_stream.next() => {
                        if let Ok(val) = change.get().await {
                            let w = window_weak.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(win) = w.upgrade() { win.set_power_savings(val as i32); }
                            }).ok();
                        }
                    }
                }
            }
        });
    }

    // Advanced tab: daemon controls.
    {
        let window_weak = window.as_weak();
        window.on_start_daemon(move || {
            let window_weak = window_weak.clone();
            tokio::spawn(async move {
                systemctl(&["start", "blacksharkd"]).await;
                let status = daemon_status().await;
                let w = window_weak.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(win) = w.upgrade() { win.set_daemon_status(status.into()); }
                }).ok();
            });
        });
    }

    {
        let window_weak = window.as_weak();
        window.on_stop_daemon(move || {
            let window_weak = window_weak.clone();
            tokio::spawn(async move {
                systemctl(&["stop", "blacksharkd"]).await;
                let status = daemon_status().await;
                let w = window_weak.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(win) = w.upgrade() { win.set_daemon_status(status.into()); }
                }).ok();
            });
        });
    }

    {
        let window_weak = window.as_weak();
        window.on_restart_daemon(move || {
            let window_weak = window_weak.clone();
            tokio::spawn(async move {
                systemctl(&["restart", "blacksharkd"]).await;
                let status = daemon_status().await;
                let w = window_weak.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(win) = w.upgrade() { win.set_daemon_status(status.into()); }
                }).ok();
            });
        });
    }

    // Advanced tab: manual sink tear-down.
    {
        let modules = modules.clone();
        let window_weak = window.as_weak();
        window.on_teardown_sinks(move || {
            let modules = modules.clone();
            let window_weak = window_weak.clone();
            tokio::spawn(async move {
                let mods: Vec<u32> = modules.lock().unwrap().drain(..).collect();
                for id in mods {
                    pipewire::unload_module(id).await;
                }
                let w = window_weak.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(win) = w.upgrade() { win.set_sinks_active(false); }
                }).ok();
            });
        });
    }

    // Advanced tab: manual sink rebuild.
    {
        let modules = modules.clone();
        let current_mix = current_mix.clone();
        let window_weak = window.as_weak();
        window.on_rebuild_sinks(move || {
            let modules = modules.clone();
            let current_mix = current_mix.clone();
            let window_weak = window_weak.clone();
            tokio::spawn(async move {
                // Tear down any existing modules first.
                let mods: Vec<u32> = modules.lock().unwrap().drain(..).collect();
                for id in mods {
                    pipewire::unload_module(id).await;
                }
                let mix = current_mix.load(Ordering::Relaxed);
                let new_mods = setup_sinks(mix).await;
                let active = !new_mods.is_empty();
                modules.lock().unwrap().extend(new_mods);
                let w = window_weak.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(win) = w.upgrade() { win.set_sinks_active(active); }
                }).ok();
            });
        });
    }

    window.run()?;

    // Tear down PipeWire sinks when the window closes.
    let mods: Vec<u32> = modules.lock().unwrap().drain(..).collect();
    for id in mods {
        pipewire::unload_module(id).await;
    }

    Ok(())
}
