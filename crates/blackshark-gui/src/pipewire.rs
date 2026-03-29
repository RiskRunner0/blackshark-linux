use anyhow::{Context, Result};
use tokio::process::Command;

// ---------------------------------------------------------------------------
// Sink-input enumeration and routing
// ---------------------------------------------------------------------------

/// A running audio stream and its current routing.
#[derive(Debug, Clone)]
pub struct SinkInput {
    pub id: u32,
    /// Human-readable application name.
    pub app_name: String,
    /// "game", "chat", or "" for anything else.
    pub route: String,
}

/// List all non-loopback audio streams currently playing.
pub async fn list_sink_inputs() -> Vec<SinkInput> {
    let sink_map = build_sink_name_map().await;

    let output = match Command::new("pactl")
        .args(["--format=json", "list", "sink-inputs"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("pactl list sink-inputs failed: {e}");
            return vec![];
        }
    };

    let json: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("failed to parse sink-inputs JSON: {e}");
            return vec![];
        }
    };

    let Some(entries) = json.as_array() else {
        return vec![];
    };

    let mut result = Vec::new();
    for entry in entries {
        if entry["client"].is_null() {
            continue;
        }

        let id = match entry["index"].as_u64() {
            Some(v) => v as u32,
            None => continue,
        };

        let props = &entry["properties"];
        let app_name = props["application.process.binary"]
            .as_str()
            .or_else(|| props["application.name"].as_str())
            .unwrap_or("Unknown")
            .to_owned();

        let sink_index = entry["sink"].as_u64().unwrap_or(0) as u32;
        let sink_name = sink_map.get(&sink_index).map(|s| s.as_str()).unwrap_or("");
        let route = if sink_name == "blackshark-game" {
            "game".to_owned()
        } else if sink_name == "blackshark-chat" {
            "chat".to_owned()
        } else {
            String::new()
        };

        result.push(SinkInput {
            id,
            app_name,
            route,
        });
    }

    result
}

/// Move a sink-input to the given sink name.
pub async fn move_sink_input(id: u32, sink_name: &str) {
    let status = Command::new("pactl")
        .args(["move-sink-input", &id.to_string(), sink_name])
        .status()
        .await;
    if let Err(e) = status {
        eprintln!("pactl move-sink-input {id} {sink_name} failed: {e}");
    }
}

async fn build_sink_name_map() -> std::collections::HashMap<u32, String> {
    let mut map = std::collections::HashMap::new();
    let output = match Command::new("pactl")
        .args(["list", "short", "sinks"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return map,
    };
    let stdout = String::from_utf8(output.stdout).unwrap_or_default();
    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(idx), Some(name)) = (parts.next(), parts.next()) {
            if let Ok(idx) = idx.parse::<u32>() {
                map.insert(idx, name.to_owned());
            }
        }
    }
    map
}

/// Create a null sink and return its pactl module ID.
pub async fn load_null_sink(name: &str, desc: &str) -> Result<u32> {
    let output = Command::new("pactl")
        .args([
            "load-module",
            "module-null-sink",
            &format!("sink_name={name}"),
            &format!("sink_properties=device.description={desc}"),
        ])
        .output()
        .await
        .context("pactl not found")?;

    let id_str = String::from_utf8(output.stdout).context("pactl output not utf8")?;
    let id: u32 = id_str
        .trim()
        .parse()
        .with_context(|| format!("unexpected pactl output: {}", id_str.trim()))?;

    eprintln!("blackshark-gui: loaded PipeWire null sink '{name}' (module {id})");
    Ok(id)
}

/// Create a loopback from `source` monitor to `sink` and return the module ID.
pub async fn load_loopback(source_monitor: &str, sink: &str) -> Result<u32> {
    let output = Command::new("pactl")
        .args([
            "load-module",
            "module-loopback",
            &format!("source={source_monitor}"),
            &format!("sink={sink}"),
            "latency_msec=1",
        ])
        .output()
        .await
        .context("pactl not found")?;

    let id_str = String::from_utf8(output.stdout).context("pactl output not utf8")?;
    let id: u32 = id_str
        .trim()
        .parse()
        .with_context(|| format!("unexpected pactl output: {}", id_str.trim()))?;

    eprintln!("blackshark-gui: loaded PipeWire loopback {source_monitor} -> {sink} (module {id})");
    Ok(id)
}

/// Unload any leftover blackshark-game / blackshark-chat modules from a
/// previous run. Called once at startup before creating new sinks.
pub async fn cleanup_stale_sinks() {
    let output = match Command::new("pactl")
        .args(["list", "short", "modules"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return,
    };

    let stdout = match String::from_utf8(output.stdout) {
        Ok(s) => s,
        Err(_) => return,
    };

    for line in stdout.lines() {
        if line.contains("blackshark-game") || line.contains("blackshark-chat") {
            if let Some(id_str) = line.split_whitespace().next() {
                if let Ok(id) = id_str.parse::<u32>() {
                    eprintln!("blackshark-gui: removing stale PipeWire module {id}");
                    unload_module(id).await;
                }
            }
        }
    }
}

/// Find the real ALSA sink name for the BlackShark V3 Pro headset.
pub async fn find_headset_sink() -> Option<String> {
    let output = Command::new("pactl")
        .args(["list", "sinks", "short"])
        .output()
        .await
        .ok()?;

    let stdout = String::from_utf8(output.stdout).ok()?;
    for line in stdout.lines() {
        let lower = line.to_lowercase();
        if lower.contains("blackshark_v3") || lower.contains("blackshark v3") {
            let name = line.split_whitespace().nth(1)?;
            return Some(name.to_owned());
        }
    }
    None
}

/// Unload a pactl module by ID.
pub async fn unload_module(id: u32) {
    let _ = Command::new("pactl")
        .args(["unload-module", &id.to_string()])
        .status()
        .await;
}

/// Set sink volumes for a game/chat crossfader position.
///
/// mix = 0   -> game 0%,   chat 100%
/// mix = 50  -> game 100%, chat 100%
/// mix = 100 -> game 100%, chat 0%
pub async fn apply_mix_volumes(mix: u8) {
    let mix = mix.min(100);
    let game_pct: u32 = if mix >= 50 { 100 } else { mix as u32 * 2 };
    let chat_pct: u32 = if mix <= 50 {
        100
    } else {
        (100 - mix as u32) * 2
    };

    for (sink, pct) in [("blackshark-game", game_pct), ("blackshark-chat", chat_pct)] {
        let _ = Command::new("pactl")
            .args(["set-sink-volume", sink, &format!("{pct}%")])
            .status()
            .await;
    }
}
