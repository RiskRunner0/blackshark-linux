use anyhow::{bail, Context, Result};
use hidapi::{HidApi, HidDevice};
use tracing::info;

use blackshark_protocol::{Report, ResponseStatus, REPORT_LEN};

const VID: u16 = 0x1532;
const PID: u16 = 0x0577;

/// Open the BlackShark V3 Pro HID device.
///
/// Must open interface 5 specifically — the dongle exposes multiple HID interfaces
/// and api.open(VID, PID) picks the first enumerated, which varies across systems.
/// Interface 5 is the proprietary control interface (interrupt IN, endpoint 0x84).
pub fn open() -> Result<HidDevice> {
    let api = HidApi::new().context("failed to initialise hidapi")?;

    let mut target = None;
    for info in api.device_list() {
        if info.vendor_id() == VID && info.product_id() == PID {
            let path = info.path().to_string_lossy();
            info!(
                interface = info.interface_number(),
                path = %path,
                "found BlackShark hidraw interface"
            );
            if info.interface_number() == 5 {
                target = Some(info.clone());
            }
        }
    }

    match target {
        None => bail!("BlackShark V3 Pro not found — is the dongle plugged in and do you have udev permission?"),
        Some(info) => {
            let path = info.path().to_string_lossy().into_owned();
            let dev = info
                .open_device(&api)
                .context("found BlackShark V3 Pro but failed to open control interface — check udev permissions")?;
            info!(path = %path, "opened BlackShark control interface");
            Ok(dev)
        }
    }
}

/// Send a report and return Ok if ANY 64-byte response arrives (regardless of status).
/// Used as a wireless link readiness probe — any response means the link is up.
pub fn send_probe(dev: &HidDevice, report: &Report) -> Result<()> {
    dev.write(report.as_bytes()).context("HID write failed")?;
    let mut buf = [0u8; REPORT_LEN];
    let n = dev
        .read_timeout(&mut buf, 2_000)
        .context("HID read failed")?;
    if n != REPORT_LEN {
        bail!("short read: expected {REPORT_LEN} bytes, got {n}");
    }
    Ok(())
}

/// Write a report without waiting for a response (fire-and-forget).
/// Used for init handshake commands where the side-effect of sending
/// matters but the response may not arrive or may be ignored.
pub fn send_no_wait(dev: &HidDevice, report: &Report) -> Result<()> {
    dev.write(report.as_bytes()).context("HID write failed")?;
    Ok(())
}

/// Send a report and read back the response.
///
/// Razer devices echo the command back with the status byte set.
pub fn send(dev: &HidDevice, report: &Report) -> Result<Report> {
    dev.write(report.as_bytes()).context("HID write failed")?;

    let mut buf = [0u8; REPORT_LEN];
    let n = dev
        .read_timeout(&mut buf, 5_000)
        .context("HID read failed")?;

    if n != REPORT_LEN {
        bail!("short read: expected {REPORT_LEN} bytes, got {n}");
    }

    let response = Report::from_bytes(buf);

    match response.status() {
        ResponseStatus::Ok => Ok(response),
        other => bail!(
            "device returned error status: {other:?} (raw=0x{:02x})",
            response.as_bytes()[1]
        ),
    }
}
