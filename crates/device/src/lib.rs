use anyhow::{bail, Context, Result};
use hidapi::{HidApi, HidDevice};

use blackshark_protocol::{Report, ResponseStatus, REPORT_LEN};

const VID: u16 = 0x1532;
const PID: u16 = 0x0577;

/// Open the BlackShark V3 Pro HID device.
pub fn open() -> Result<HidDevice> {
    let api = HidApi::new().context("failed to initialise hidapi")?;
    api.open(VID, PID)
        .context("failed to open BlackShark V3 Pro — is it connected and do you have permission?")
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
        other => bail!("device returned error status: {other:?}"),
    }
}
