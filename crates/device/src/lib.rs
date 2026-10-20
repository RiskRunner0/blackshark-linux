use anyhow::{bail, Context, Result};
use hidapi::{HidApi, HidDevice};
use std::time::{Duration, Instant};

use tracing::{debug, info};

use blackshark_protocol::{Report, ResponseStatus, REPORT_LEN};

const VID: u16 = 0x1532;

/// Supported dongles as (USB product ID: u16, proprietary control interface: u8).
const DEVICES: [(u16, u8); 3] = [
    (0x0577, 5), // BlackShark V3 Pro
    (0x0a55, 5), // BlackShark V3 Pro for Xbox
    (0x0565, 3), // BlackShark V2 HS
];

/// Open the BlackShark HID device.
///
/// Must open the proprietary control interface specifically (Interface 5 for V3 Pro, Interface 3 for V2 HS)
/// — the dongle exposes multiple HID interfaces and api.open(VID, PID) picks the first
/// enumerated, which varies across systems.
pub fn open() -> Result<HidDevice> {
    let api = HidApi::new().context("failed to initialise hidapi")?;

    let mut target = None;
    let mut seen = Vec::new();
    for info in api.device_list() {
        if info.vendor_id() == VID {
            let pid = info.product_id();
            let Some((_, target_interface)) =
                DEVICES.iter().find(|(device_pid, _)| *device_pid == pid)
            else {
                continue;
            };
            let path = info.path().to_string_lossy();
            debug!(
                interface = info.interface_number(),
                path = %path,
                pid = pid,
                "found BlackShark hidraw interface"
            );
            seen.push(format!("{}=if{}", path, info.interface_number()));
            if info.interface_number() == i32::from(*target_interface) {
                target = Some(info.clone());
            }
        }
    }

    match target {
        None if seen.is_empty() => bail!(
            "BlackShark headset (1532:0577, 1532:0a55 Xbox, or 1532:0565 V2 HS) not visible to hidapi — is the dongle plugged in, \
             and does your user have access to /dev/hidraw* (udev rule installed, dongle replugged)?"
        ),
        None => bail!(
            "BlackShark headset found but has no control HID interface (saw: {}) — \
             firmware may use a different interface layout; please report this",
            seen.join(", ")
        ),
        Some(info) => {
            let path = info.path().to_string_lossy().into_owned();
            let dev = info.open_device(&api).with_context(|| {
                format!("found {path} but failed to open it — check udev permissions (ls -l {path})")
            })?;
            info!(
                path = %path,
                release = format_args!("{:#06x}", info.release_number()),
                product = info.product_string().unwrap_or("?"),
                "opened BlackShark control interface"
            );
            Ok(dev)
        }
    }
}

/// The device accepted the write but sent no matching response within the timeout.
#[derive(Debug)]
pub struct DeviceStatusError {
    pub status: ResponseStatus,
    pub raw: u8,
}

impl std::fmt::Display for DeviceStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "device returned error status: {:?} (raw=0x{:02x})",
            self.status, self.raw
        )
    }
}

impl std::error::Error for DeviceStatusError {}

/// Send a report and read back the response.
///
/// Distinct from an I/O error: the hidraw handle is still valid, but the dongle's
/// wireless link to the headset is not up (headset off, unpaired, or on Bluetooth).
#[derive(Debug)]
pub struct NoResponse;

impl std::fmt::Display for NoResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no response from dongle")
    }
}

impl std::error::Error for NoResponse {}

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);
const BUSY_RETRIES: u32 = 3;

/// Discard any reports already queued on the interrupt IN endpoint.
///
/// The headset pushes unsolicited reports (e.g. charging-state changes), which would
/// otherwise be read as the reply to the next command.
fn drain(dev: &HidDevice) {
    let mut buf = [0u8; REPORT_LEN];
    while matches!(dev.read_timeout(&mut buf, 0), Ok(n) if n > 0) {}
}

/// Read until a response to `request` arrives (same class, sub=0x01), skipping
/// unsolicited pushes and malformed reports. `Ok(None)` means the deadline passed.
fn read_response(dev: &HidDevice, request: &Report) -> Result<Option<Report>> {
    let class = request.as_bytes()[10];
    let deadline = Instant::now() + RESPONSE_TIMEOUT;
    let mut buf = [0u8; REPORT_LEN];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        let n = dev
            .read_timeout(&mut buf, remaining.as_millis().max(1) as i32)
            .context("HID read failed")?;
        if n == 0 {
            return Ok(None);
        }
        if n != REPORT_LEN {
            debug!(len = n, "ignoring short report");
            continue;
        }
        if buf[10] == class && buf[11] == 0x01 {
            return Ok(Some(Report::from_bytes(buf)));
        }
        debug!(class = buf[10], sub = buf[11], "ignoring unrelated report");
    }
}

/// Send a report and read back the matching response.
///
/// Errors: an `NoResponse` source means the link is silent; any other error is an
/// I/O failure (device unplugged) or a device-reported failure status.
pub fn send(dev: &HidDevice, report: &Report) -> Result<Report> {
    for _ in 0..BUSY_RETRIES {
        drain(dev);
        dev.write(report.as_bytes()).context("HID write failed")?;
        let Some(response) = read_response(dev, report)? else {
            return Err(NoResponse.into());
        };
        match response.status() {
            ResponseStatus::Ok => return Ok(response),
            ResponseStatus::Busy => std::thread::sleep(Duration::from_millis(150)),
            other => {
                return Err(anyhow::Error::new(DeviceStatusError {
                    status: other,
                    raw: response.as_bytes()[1],
                }));
            }
        }
    }
    bail!("device busy")
}
