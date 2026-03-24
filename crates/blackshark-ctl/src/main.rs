use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;
use zbus::Connection;

mod proxy;
use proxy::HeadsetProxy;

#[derive(Parser)]
#[command(name = "blackshark-ctl", about = "Control the Razer BlackShark V3 Pro headset")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Set sidetone level (0–15)
    Sidetone {
        #[arg(value_name = "LEVEL", value_parser = clap::value_parser!(u8).range(0..=15))]
        level: u8,
    },
    /// Query battery level
    Battery,
    /// Print full device status as JSON (useful for waybar / scripts)
    Status,
    /// Subscribe to all device signals and print changes as they arrive
    Monitor,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let conn = Connection::session().await?;
    let proxy = HeadsetProxy::new(&conn).await?;

    // monitor doesn't need the headset to be connected — it just watches
    if !matches!(cli.command, Command::Monitor) && !proxy.connected().await? {
        bail!("headset is not connected (is blacksharkd running?)");
    }

    match cli.command {
        Command::Sidetone { level } => {
            proxy.set_sidetone(level).await?;
            println!("sidetone set to {level}");
        }
        Command::Battery => {
            let (pct, charging) = proxy.get_battery().await?;
            let charging = if charging { " (charging)" } else { "" };
            println!("battery: {pct}%{charging}");
        }
        Command::Status => cmd_status(&proxy).await?,
        Command::Monitor => cmd_monitor(&proxy).await?,
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// status --json
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Status {
    connected: bool,
    battery_percentage: u8,
    sidetone: u8,
}

async fn cmd_status(proxy: &HeadsetProxy<'_>) -> Result<()> {
    let status = Status {
        connected:          proxy.connected().await?,
        battery_percentage: proxy.battery_percentage().await?,
        sidetone:           proxy.sidetone().await?,
    };
    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}

// ---------------------------------------------------------------------------
// monitor
// ---------------------------------------------------------------------------

async fn cmd_monitor(proxy: &HeadsetProxy<'_>) -> Result<()> {
    use futures_util::StreamExt;

    eprintln!("monitoring — press Ctrl+C to stop");

    // Print current state first so there's always a baseline.
    if proxy.connected().await? {
        let (pct, charging) = proxy.get_battery().await?;
        println!("connected  battery={}% charging={} sidetone={}",
            pct, charging, proxy.sidetone().await?);
    } else {
        println!("disconnected");
    }

    let mut battery_stream   = proxy.receive_battery_changed().await?;
    let mut connected_stream = proxy.receive_connected_changed().await;
    let mut sidetone_stream  = proxy.receive_sidetone_changed().await;

    loop {
        tokio::select! {
            Some(sig) = battery_stream.next() => {
                let args = sig.args()?;
                println!("battery_changed  percentage={}  charging={}", args.percentage, args.charging);
            }
            Some(change) = connected_stream.next() => {
                let val = change.get().await?;
                println!("connected_changed  connected={val}");
            }
            Some(change) = sidetone_stream.next() => {
                let val = change.get().await?;
                println!("sidetone_changed  sidetone={val}");
            }
        }
    }
}
