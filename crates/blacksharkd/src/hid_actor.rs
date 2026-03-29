use std::time::{Duration, Instant};

use anyhow::Result;
use hidapi::HidDevice;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{debug, info, warn};

use blackshark_device as device;
use blackshark_protocol::{cmd, Report};

use crate::config::Config;
use crate::state::SharedState;

// ---------------------------------------------------------------------------
// Public command API
// ---------------------------------------------------------------------------

pub struct BatteryState {
    pub percentage: u8,
    pub charging: bool,
}

pub enum HidCommand {
    SetSidetone {
        level: u8,
        reply: oneshot::Sender<Result<()>>,
    },
    GetBattery {
        reply: oneshot::Sender<Result<BatteryState>>,
    },
    SetThx {
        enabled: bool,
        reply: oneshot::Sender<Result<()>>,
    },
    SetAnc {
        enabled: bool,
        level: u8,
        reply: oneshot::Sender<Result<()>>,
    },
    SetPowerSavings {
        minutes: u8,
        reply: oneshot::Sender<Result<()>>,
    },
    SetEq {
        preset: u8,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Sent when config changes — restores all settings to the device.
    ApplyConfig { config: Config },
    /// Periodic wakeup sent by a tokio timer — drives reconnect + battery poll.
    Tick,
}

// ---------------------------------------------------------------------------
// Actor entry point
// ---------------------------------------------------------------------------

const BATTERY_POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Spawn the HID actor on a dedicated OS thread.
///
/// `HidDevice` is not `Send`, so all HID I/O stays on this thread.
/// Communication with async callers is via the mpsc channel + oneshot replies.
pub fn spawn(
    rx: mpsc::Receiver<HidCommand>,
    state_tx: watch::Sender<SharedState>,
    initial_config: Config,
) {
    std::thread::Builder::new()
        .name("hid-actor".into())
        .spawn(move || run(rx, state_tx, initial_config))
        .expect("failed to spawn hid-actor thread");
}

fn run(
    mut rx: mpsc::Receiver<HidCommand>,
    state_tx: watch::Sender<SharedState>,
    initial_config: Config,
) {
    let mut dev: Option<HidDevice> = try_open();
    let mut next_battery_poll = Instant::now(); // poll immediately on first tick
    let mut device_ready = false; // true after first successful battery poll

    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            HidCommand::Tick => {
                if dev.is_none() {
                    if let Some(d) = try_open() {
                        dev = Some(d);
                        device_ready = false;
                    }
                }
                if Instant::now() >= next_battery_poll {
                    if let Some(d) = &dev {
                        match query_battery(d) {
                            Ok(b) => {
                                next_battery_poll = Instant::now() + BATTERY_POLL_INTERVAL;
                                debug!(
                                    percentage = b.percentage,
                                    charging = b.charging,
                                    "battery poll"
                                );
                                if !device_ready {
                                    // First successful battery poll = wireless link established.
                                    device_ready = true;
                                    let sidetone = query_sidetone(d).ok();
                                    info!(percentage = b.percentage, sidetone, "headset connected");
                                    state_tx.send_modify(|s| {
                                        s.connected = true;
                                        s.battery_pct = b.percentage;
                                        s.charging = b.charging;
                                        if let Some(v) = sidetone {
                                            s.sidetone = v;
                                        }
                                    });
                                    restore_config(d, &initial_config);
                                } else {
                                    state_tx.send_modify(|s| {
                                        s.battery_pct = b.percentage;
                                        s.charging = b.charging;
                                    });
                                }
                            }
                            Err(e) => {
                                if device_ready {
                                    warn!("headset disconnected: {e}");
                                    device_ready = false;
                                    dev = None;
                                    state_tx.send_modify(|s| s.connected = false);
                                } else {
                                    debug!("waiting for RF link: {e}");
                                }
                            }
                        }
                    }
                }
            }

            HidCommand::SetSidetone { level, reply } => {
                info!(level, "set_sidetone");
                let result = with_dev(&mut dev, &state_tx, |d| set_sidetone(d, level));
                match &result {
                    Ok(()) => {
                        info!(level, "set_sidetone ok");
                        state_tx.send_modify(|s| s.sidetone = level);
                    }
                    Err(e) => warn!("set_sidetone failed: {e}"),
                }
                let _ = reply.send(result);
            }

            HidCommand::SetThx { enabled, reply } => {
                info!(enabled, "set_thx");
                let result = with_dev(&mut dev, &state_tx, |d| set_thx(d, enabled));
                match &result {
                    Ok(()) => {
                        info!(enabled, "set_thx ok");
                        state_tx.send_modify(|s| s.thx_enabled = enabled);
                    }
                    Err(e) => warn!("set_thx failed: {e}"),
                }
                let _ = reply.send(result);
            }

            HidCommand::SetAnc {
                enabled,
                level,
                reply,
            } => {
                info!(enabled, level, "set_anc");
                let result = with_dev(&mut dev, &state_tx, |d| set_anc(d, enabled, level));
                match &result {
                    Ok(()) => {
                        info!(enabled, level, "set_anc ok");
                        state_tx.send_modify(|s| {
                            s.anc_enabled = enabled;
                            s.anc_level = level;
                        });
                    }
                    Err(e) => warn!("set_anc failed: {e}"),
                }
                let _ = reply.send(result);
            }

            HidCommand::SetPowerSavings { minutes, reply } => {
                info!(minutes, "set_power_savings");
                let result = with_dev(&mut dev, &state_tx, |d| set_power_savings(d, minutes));
                match &result {
                    Ok(()) => {
                        info!(minutes, "set_power_savings ok");
                        state_tx.send_modify(|s| s.power_savings_minutes = minutes);
                    }
                    Err(e) => warn!("set_power_savings failed: {e}"),
                }
                let _ = reply.send(result);
            }

            HidCommand::SetEq { preset, reply } => {
                info!(preset, "set_eq");
                let result = with_dev(&mut dev, &state_tx, |d| set_eq_preset(d, preset));
                match &result {
                    Ok(()) => {
                        info!(preset, "set_eq ok");
                        state_tx.send_modify(|s| s.eq_preset = preset);
                    }
                    Err(e) => warn!("set_eq failed: {e}"),
                }
                let _ = reply.send(result);
            }

            HidCommand::GetBattery { reply } => {
                info!("get_battery");
                let result = with_dev(&mut dev, &state_tx, query_battery);
                match &result {
                    Ok(b) => {
                        info!(
                            percentage = b.percentage,
                            charging = b.charging,
                            "get_battery ok"
                        );
                        state_tx.send_modify(|s| {
                            s.battery_pct = b.percentage;
                            s.charging = b.charging;
                        });
                    }
                    Err(e) => warn!("get_battery failed: {e}"),
                }
                let _ = reply.send(result);
            }

            HidCommand::ApplyConfig { config } => {
                if let Some(d) = &dev {
                    restore_config(d, &config);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Send the initialization handshake that the Windows Razer driver issues on connect.
///
/// The dongle's HID config interface only becomes active once the 2.4GHz wireless
/// link to the headset is established — typically 10–15 seconds after USB plug-in.
/// Windows waits for this naturally (driver + Synapse startup takes that long).
/// We must poll until the device responds before proceeding with any config commands.
///
/// Sequence observed in bare-metal Windows usbmon capture:
///   1. cls=0x02, flag=0x00 — device capability query (response = device is ready)
///   2. cls=0x2a, flag=0x00 — charging capability query
///   Then normal queries follow.
///
/// Placeholder — the RF link establishes automatically without init commands.
/// Sending commands before the link is up pollutes the read buffer and causes
/// subsequent battery reads to consume stale responses. We do nothing here and
/// let the Tick's battery poll detect when the link is ready.
fn init_session(_dev: &HidDevice) {}

/// Open the device and optionally restore config immediately.
/// Open the hidraw device and fire the init handshake.
/// Does NOT wait for the wireless RF link — that may take ~44s after a cold replug.
/// The Tick handler will detect readiness via battery poll and restore config then.
fn try_open() -> Option<HidDevice> {
    match device::open() {
        Err(_) => None,
        Ok(d) => {
            init_session(&d);
            info!("dongle opened, waiting for wireless link");
            Some(d)
        }
    }
}

/// Apply all config values to the device. Logs but does not fail on errors —
/// best-effort restore so a single bad command doesn't block the rest.
fn restore_config(dev: &HidDevice, config: &Config) {
    info!(
        sidetone = config.sidetone,
        thx = config.thx_enabled,
        anc = config.anc_enabled,
        power_savings = config.power_savings_minutes,
        "restoring config to device"
    );

    if let Err(e) = set_sidetone(dev, config.sidetone) {
        warn!("restore sidetone failed: {e}");
    }
    if config.eq_preset > 0 {
        if let Err(e) = set_eq_preset(dev, config.eq_preset) {
            warn!("restore eq failed: {e}");
        }
    }
}

/// Run `f` with the current device, clearing it on I/O failure.
fn with_dev<T, F>(
    dev: &mut Option<HidDevice>,
    state_tx: &watch::Sender<SharedState>,
    f: F,
) -> Result<T>
where
    F: FnOnce(&HidDevice) -> Result<T>,
{
    match dev {
        None => anyhow::bail!("headset not connected"),
        Some(d) => {
            let result = f(d);
            if result.is_err() {
                warn!("headset disconnected");
                *dev = None;
                state_tx.send_modify(|s| s.connected = false);
            }
            result
        }
    }
}

// ---------------------------------------------------------------------------
// HID operations
// ---------------------------------------------------------------------------

fn set_sidetone(dev: &HidDevice, level: u8) -> Result<()> {
    let get = Report::new(
        0x60,
        cmd::SIDETONE_GET_CLASS,
        cmd::SIDETONE_ID,
        &[cmd::SIDETONE_GET_ARG, 0x00],
    );
    device::send(dev, &get)?;
    let set = Report::new(
        0x60,
        cmd::SIDETONE_SET_CLASS,
        cmd::SIDETONE_ID,
        &[level, 0x00],
    );
    device::send(dev, &set)?;
    Ok(())
}

fn query_battery(dev: &HidDevice) -> Result<BatteryState> {
    let report = Report::new(0x60, cmd::BATTERY_CLASS, cmd::BATTERY_ID, &[0x00]);
    let response = device::send(dev, &report)?;
    let args = response.args();
    anyhow::ensure!(args.len() >= 2, "battery response too short");
    anyhow::ensure!(
        args[0] <= 100,
        "battery percentage out of range: {}",
        args[0]
    );
    Ok(BatteryState {
        percentage: args[0],
        charging: args[1] != 0x00,
    })
}

fn set_thx(dev: &HidDevice, enabled: bool) -> Result<()> {
    let mode = if enabled {
        cmd::THX_SPATIAL
    } else {
        cmd::THX_STEREO
    };
    let report = Report::new(0x60, cmd::THX_CLASS, cmd::THX_ID, &[mode, 0x00]);
    device::send(dev, &report)?;
    Ok(())
}

fn set_anc(dev: &HidDevice, enabled: bool, level: u8) -> Result<()> {
    let level = level.clamp(cmd::ANC_LEVEL_MIN, cmd::ANC_LEVEL_MAX);
    let report = Report::new(
        0x60,
        cmd::ANC_CLASS,
        cmd::ANC_ID,
        &[enabled as u8, level, 0x00],
    );
    device::send(dev, &report)?;
    Ok(())
}

fn set_power_savings(dev: &HidDevice, minutes: u8) -> Result<()> {
    let report = Report::new(
        0x60,
        cmd::POWER_SAVINGS_CLASS,
        cmd::POWER_SAVINGS_ID,
        &[minutes, 0x00],
    );
    device::send(dev, &report)?;
    Ok(())
}

/// Band data per preset (confirmed from Synapse pcap captures).
/// Format: [preset_idx, b0..b8, extra, padding] — 12 bytes total.
/// Band values use sign-magnitude encoding: 0x00=0dB, 0x01=+1dB, 0x81=−1dB.
/// Bands: 60Hz, 170Hz, 310Hz, 600Hz, 1kHz, 3kHz, 6kHz, 12kHz, 16kHz.
const EQ_BANDS: [[u8; 12]; 5] = [
    [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ], // 0: Flat
    [
        0x01, 0x02, 0x02, 0x05, 0x05, 0x01, 0x81, 0x02, 0x03, 0x03, 0x03, 0x00,
    ], // 1
    [
        0x02, 0x03, 0x03, 0x03, 0x81, 0x84, 0x84, 0x02, 0x03, 0x03, 0x03, 0x00,
    ], // 2
    [
        0x03, 0x02, 0x02, 0x00, 0x00, 0x01, 0x81, 0x81, 0x03, 0x03, 0x03, 0x00,
    ], // 3
    [
        0x04, 0x01, 0x01, 0x81, 0x00, 0x02, 0x00, 0x04, 0x04, 0x04, 0x83, 0x00,
    ], // 4
];

/// Meta args per preset (7 bytes, from captures).
const EQ_META: [[u8; 7]; 5] = [
    [0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
    [0x01, 0x01, 0x01, 0x00, 0x01, 0x00, 0x00],
    [0x02, 0x03, 0x01, 0x00, 0x03, 0x00, 0x00],
    [0x03, 0x02, 0x01, 0x00, 0x02, 0x00, 0x00],
    [0x04, 0x04, 0x01, 0x00, 0x0b, 0x00, 0x00],
];

/// Commit args per preset (12 bytes, from captures).
const EQ_COMMIT: [[u8; 12]; 5] = [
    [
        0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ],
    [
        0x01, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ],
    [
        0x02, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ],
    [
        0x03, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ],
    [
        0x04, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ],
];

fn set_eq_preset(dev: &HidDevice, preset: u8) -> Result<()> {
    anyhow::ensure!(
        preset < cmd::EQ_PRESET_COUNT,
        "preset index out of range (0–8)"
    );

    // Only presets 0–4 have captured data; 5–8 use flat band data with their index.
    let idx = preset as usize;
    let (bands, meta, commit) = if idx < EQ_BANDS.len() {
        (EQ_BANDS[idx], EQ_META[idx], EQ_COMMIT[idx])
    } else {
        let mut b = EQ_BANDS[0];
        b[0] = preset;
        let mut m = EQ_META[0];
        m[0] = preset;
        let mut c = EQ_COMMIT[0];
        c[0] = preset;
        (b, m, c)
    };

    // 1. GET current state
    device::send(
        dev,
        &Report::new(0x60, cmd::EQ_STATE_CLASS, cmd::EQ_STATE_ID, &[0x01, 0x00]),
    )?;

    // 2. SET bands
    device::send(
        dev,
        &Report::new(0x60, cmd::EQ_BANDS_CLASS, cmd::EQ_BANDS_ID, &bands),
    )?;

    // 3. SET meta
    device::send(
        dev,
        &Report::new(0x60, cmd::EQ_META_CLASS, cmd::EQ_META_ID, &meta),
    )?;

    // 4. APPLY
    device::send(
        dev,
        &Report::new(0x60, cmd::EQ_STATE_CLASS, cmd::EQ_STATE_ID, &[0x02, 0x00]),
    )?;

    // 5. COMMIT
    device::send(
        dev,
        &Report::new(0x60, cmd::EQ_COMMIT_CLASS, cmd::EQ_COMMIT_ID, &commit),
    )?;

    Ok(())
}

fn query_sidetone(dev: &HidDevice) -> Result<u8> {
    let report = Report::new(0x60, cmd::SIDETONE_READ_CLASS, 0x00, &[0x00]);
    let response = device::send(dev, &report)?;
    let args = response.args();
    anyhow::ensure!(!args.is_empty(), "sidetone response empty");
    anyhow::ensure!(
        args[0] <= cmd::SIDETONE_MAX,
        "sidetone out of range: {}",
        args[0]
    );
    Ok(args[0])
}
