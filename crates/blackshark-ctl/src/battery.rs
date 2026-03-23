use anyhow::{bail, Context, Result};
use blackshark_device as device;
use blackshark_protocol::{cmd, Report};

pub struct BatteryState {
    pub percentage: u8,
    pub charging: bool,
}

pub fn query(dev: &hidapi::HidDevice) -> Result<BatteryState> {
    let report = Report::new(0x60, cmd::BATTERY_CLASS, cmd::BATTERY_ID, &[0x00]);
    let response = device::send(dev, &report).context("battery query failed")?;

    let args = response.args();
    if args.len() < 2 {
        bail!("battery response too short: got {} bytes, expected 2", args.len());
    }

    Ok(BatteryState {
        percentage: args[0],
        charging:   args[1] != 0x00,
    })
}
