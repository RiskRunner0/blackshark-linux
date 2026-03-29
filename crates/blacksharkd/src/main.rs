mod config;
mod dbus;
mod hid_actor;
mod pipewire;
mod state;

use std::time::Duration;

use anyhow::Result;
use std::collections::HashMap;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
use zbus::fdo::Properties;
use zbus::zvariant::Value;
use zbus::ConnectionBuilder;

use config::Config;
use state::SharedState;

const TICK_INTERVAL: Duration = Duration::from_secs(5);
const DEBOUNCE_INTERVAL: Duration = Duration::from_millis(500);
const ROUTING_INTERVAL: Duration = Duration::from_secs(3);
const DBUS_PATH: &str = "/net/blackshark1/Headset";
const DBUS_NAME: &str = "net.blackshark1";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "blacksharkd=info".into()),
        )
        .init();

    // Load config from disk (or create defaults).
    let initial_config = match config::load() {
        Ok(c) => {
            info!(path = %config::config_path().unwrap().display(), "loaded config");
            c
        }
        Err(e) => {
            warn!("could not load config, using defaults: {e}");
            Config::default()
        }
    };

    // Config watch channel — D-Bus methods send updated configs here.
    let (config_tx, mut config_rx) = watch::channel(initial_config.clone());

    let (cmd_tx, cmd_rx) = mpsc::channel::<hid_actor::HidCommand>(32);
    let initial_state = SharedState {
        game_chat_mix: initial_config.game_chat_mix,
        ..SharedState::default()
    };
    let (state_tx, state_rx) = watch::channel(initial_state);

    // Spawn HID actor. Pass initial config so it can restore on first connect.
    hid_actor::spawn(cmd_rx, state_tx.clone(), initial_config);

    // Periodic tick → drives reconnect attempts and battery polling.
    let tick_tx = cmd_tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        loop {
            interval.tick().await;
            if tick_tx.send(hid_actor::HidCommand::Tick).await.is_err() {
                break;
            }
        }
    });

    // Debounced config writer + apply-to-device task.
    // Watches for config changes, waits 500ms of quiet, then saves to disk
    // and tells the HID actor to apply the new values.
    let apply_tx = cmd_tx.clone();
    tokio::spawn(async move {
        loop {
            if config_rx.changed().await.is_err() {
                break;
            }
            // Debounce: wait for changes to settle.
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(DEBOUNCE_INTERVAL) => break,
                    res = config_rx.changed() => {
                        if res.is_err() { return; }
                        // changed again — reset the timer
                    }
                }
            }
            let cfg = config_rx.borrow().clone();
            if let Err(e) = config::save(&cfg) {
                warn!("failed to save config: {e}");
            } else {
                info!("config saved");
            }
            let _ = apply_tx.send(hid_actor::HidCommand::ApplyConfig { config: cfg }).await;
        }
    });

    // D-Bus service.
    let iface = dbus::HeadsetInterface::new(cmd_tx, state_rx.clone(), state_tx.clone(), config_tx.clone());

    let conn = ConnectionBuilder::session()?
        .name(DBUS_NAME)?
        .serve_at(DBUS_PATH, iface)?
        .build()
        .await?;

    info!("running on {DBUS_NAME}");

    // Manage PipeWire virtual sinks + loopbacks for game/chat mix.
    // On connect: create null sinks and loopback each to the real headset output.
    // On disconnect: tear them all down.
    {
        let mut sink_rx = state_rx.clone();
        tokio::spawn(async move {
            let mut modules: Vec<u32> = Vec::new();

            async fn setup(modules: &mut Vec<u32>, mix: u8) {
                pipewire::cleanup_stale_sinks().await;
                let Some(headset_sink) = pipewire::find_headset_sink().await else {
                    warn!("could not find headset sink — game/chat mix unavailable");
                    return;
                };
                info!("routing game/chat mix through {headset_sink}");

                for (name, desc) in [
                    ("blackshark-game", "BlackShark-Game"),
                    ("blackshark-chat", "BlackShark-Chat"),
                ] {
                    match pipewire::load_null_sink(name, desc).await {
                        Ok(id) => modules.push(id),
                        Err(e) => { warn!("failed to create {name} sink: {e}"); return; }
                    }
                    let monitor = format!("{name}.monitor");
                    match pipewire::load_loopback(&monitor, &headset_sink).await {
                        Ok(id) => modules.push(id),
                        Err(e) => warn!("failed to create {name} loopback: {e}"),
                    }
                }

                pipewire::apply_mix_volumes(mix).await;
            }

            async fn teardown(modules: &mut Vec<u32>) {
                for id in modules.drain(..) {
                    pipewire::unload_module(id).await;
                }
            }

            // Handle case where headset is already connected at daemon startup.
            {
                let state = sink_rx.borrow_and_update().clone();
                if state.connected {
                    setup(&mut modules, state.game_chat_mix).await;
                }
            }

            loop {
                if sink_rx.changed().await.is_err() { break; }
                let state = sink_rx.borrow_and_update().clone();

                match (modules.is_empty(), state.connected) {
                    (true, true)  => setup(&mut modules, state.game_chat_mix).await,
                    (false, false) => teardown(&mut modules).await,
                    _ => {}
                }
            }
        });
    }

    // Auto-routing task: every 3 seconds, move any new sink-inputs that
    // match a saved rule to their configured sink.
    {
        let state_rx = state_rx.clone();
        let config_rx = config_tx.clone().subscribe();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(ROUTING_INTERVAL);
            loop {
                interval.tick().await;
                if !state_rx.borrow().connected { continue; }
                let cfg = config_rx.borrow().clone();
                if cfg.app_routing.is_empty() { continue; }

                let inputs = pipewire::list_sink_inputs().await;
                for input in &inputs {
                    let Some(rule) = cfg.app_routing.get(&input.app_name) else { continue };
                    let target = rule.as_str();
                    if input.route != target {
                        let sink = format!("blackshark-{target}");
                        pipewire::move_sink_input(input.id, &sink).await;
                    }
                }
            }
        });
    }

    // Watch state changes and emit D-Bus signals + PropertiesChanged.
    let mut watch_rx = state_rx;
    let conn2 = conn.clone();
    tokio::spawn(async move {
        let mut prev = state::SharedState::default();
        loop {
            if watch_rx.changed().await.is_err() {
                break;
            }
            let state = watch_rx.borrow().clone();
            let iface_ref = conn2
                .object_server()
                .interface::<_, dbus::HeadsetInterface>(DBUS_PATH)
                .await;
            let Ok(iface_ref) = iface_ref else { continue };
            let ctxt = iface_ref.signal_context();

            // Battery signal
            if state.connected && state.battery_pct != prev.battery_pct {
                dbus::HeadsetInterface::battery_changed(
                    ctxt, state.battery_pct, state.charging,
                ).await.ok();
            }

            // Emit PropertiesChanged for any state that changed.
            // This drives receive_*_changed() streams in proxy subscribers (tray, GUI).
            let mut changed: HashMap<&str, &Value<'_>> = HashMap::new();
            // Build owned values first, then reference them
            let v_connected   = Value::from(state.connected);
            let v_battery     = Value::from(state.battery_pct);
            let v_sidetone    = Value::from(state.sidetone);
            let v_eq          = Value::from(state.eq_preset);
            let v_thx         = Value::from(state.thx_enabled);
            let v_anc         = Value::from(state.anc_enabled);
            let v_anc_level   = Value::from(state.anc_level);
            let v_ps          = Value::from(state.power_savings_minutes);
            let v_mix         = Value::from(state.game_chat_mix);
            if state.connected != prev.connected           { changed.insert("Connected",           &v_connected); }
            if state.battery_pct != prev.battery_pct       { changed.insert("BatteryPercentage",    &v_battery); }
            if state.sidetone != prev.sidetone             { changed.insert("Sidetone",             &v_sidetone); }
            if state.eq_preset != prev.eq_preset           { changed.insert("EqPreset",             &v_eq); }
            if state.thx_enabled != prev.thx_enabled       { changed.insert("ThxEnabled",           &v_thx); }
            if state.anc_enabled != prev.anc_enabled       { changed.insert("AncEnabled",           &v_anc); }
            if state.anc_level != prev.anc_level           { changed.insert("AncLevel",             &v_anc_level); }
            if state.power_savings_minutes != prev.power_savings_minutes {
                changed.insert("PowerSavingsMinutes", &v_ps);
            }
            if state.game_chat_mix != prev.game_chat_mix   { changed.insert("GameChatMix",          &v_mix); }
            if !changed.is_empty() {
                Properties::properties_changed(
                    ctxt,
                    "net.blackshark1.Headset".try_into().unwrap(),
                    &changed,
                    &[],
                ).await.ok();
            }

            prev = state;
        }
    });

    std::future::pending::<()>().await;
    Ok(())
}
