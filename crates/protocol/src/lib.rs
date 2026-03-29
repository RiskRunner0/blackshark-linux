/// Razer BlackShark V3 Pro HID report: 64 bytes.
///
/// Layout (confirmed via usbmon capture with Razer Synapse on Windows):
///   [0]     Report ID       (0x02)
///   [1]     Status          (0x00 = new cmd; 0x02 = ok in response)
///   [2]     Transaction ID  (arbitrary; echoed back in response)
///   [3..8]  Padding/flags   (0x00 0x00 0x00 0x00 0x00 0x80)
///   [9]     Flags           (0x80)
///   [10]    Command class
///   [11]    Sub             (0x00)
///   [12]    Command ID
///   [13..]  Arguments       (data_size − 3 bytes; data_size counts [10..12] + args)
///   [62]    CRC             (XOR of bytes [0..61])
///   [63]    Reserved        (0x00)
pub const REPORT_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report([u8; REPORT_LEN]);

impl Report {
    pub fn new(transaction_id: u8, class: u8, id: u8, args: &[u8]) -> Self {
        Self::new_with_flags(transaction_id, 0x80, class, id, args)
    }

    /// Like `new` but with an explicit flags byte at position [9].
    /// Most commands use flags=0x80. The session init handshake uses flags=0x00.
    pub fn new_with_flags(transaction_id: u8, flags: u8, class: u8, id: u8, args: &[u8]) -> Self {
        assert!(args.len() <= 49, "argument data exceeds report capacity");

        let mut buf = [0u8; REPORT_LEN];
        buf[0] = 0x02;
        buf[1] = 0x00;
        buf[2] = transaction_id;
        buf[9] = flags;
        buf[10] = class;
        buf[11] = 0x00;
        buf[12] = id;
        let data_size = 3 + args.len();
        buf[6] = data_size as u8;
        buf[13..13 + args.len()].copy_from_slice(args);
        buf[62] = crc(&buf);
        Self(buf)
    }

    pub fn from_bytes(buf: [u8; REPORT_LEN]) -> Self {
        Self(buf)
    }

    pub fn as_bytes(&self) -> &[u8; REPORT_LEN] {
        &self.0
    }

    pub fn status(&self) -> ResponseStatus {
        ResponseStatus::from(self.0[1])
    }

    /// Argument bytes from the response (bytes [13..13+args_len]).
    pub fn args(&self) -> &[u8] {
        let data_size = self.0[6] as usize;
        let args_len = data_size.saturating_sub(3); // subtract class + sub + id
        &self.0[13..13 + args_len.min(49)]
    }
}

/// CRC is XOR of all bytes [0..61], stored at [62].
fn crc(buf: &[u8; REPORT_LEN]) -> u8 {
    buf[..62].iter().fold(0u8, |acc, &b| acc ^ b)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseStatus {
    Ok,
    Busy,
    Fail,
    Timeout,
    Unknown(u8),
}

impl From<u8> for ResponseStatus {
    fn from(b: u8) -> Self {
        match b {
            0x02 => Self::Ok,
            0x03 => Self::Busy,
            0x04 => Self::Fail,
            0x05 => Self::Timeout,
            other => Self::Unknown(other),
        }
    }
}

// ---------------------------------------------------------------------------
// Known commands (confirmed from usbmon captures with Razer Synapse)
// ---------------------------------------------------------------------------

pub mod cmd {
    /// Sidetone / mic monitoring level (0x00–0x0f, maps 1:1 to the UI range 0–15).
    ///
    /// Note: Synapse exposes a single "Sidetone" slider for this — there is no
    /// separate mic monitoring control on the V3 Pro.
    ///
    /// GET: class=0x98, id=0x01, args=[0x01, 0x00]   ← 2 arg bytes required
    /// SET: class=0x99, id=0x01, args=[level, 0x00]  ← 2 arg bytes required
    pub const SIDETONE_GET_CLASS: u8 = 0x98;
    pub const SIDETONE_SET_CLASS: u8 = 0x99;
    pub const SIDETONE_ID: u8 = 0x01;
    pub const SIDETONE_GET_ARG: u8 = 0x01;
    pub const SIDETONE_MAX: u8 = 0x0f;

    /// EQ — 5-command sequence per preset switch (confirmed from pcap).
    ///
    /// Band values use sign-magnitude encoding:
    ///   0x00 = 0dB, 0x01 = +1dB, 0x81 = −1dB, 0x84 = −4dB
    /// 9 bands: 60, 170, 310, 600, 1k, 3k, 6k, 12k, 16k Hz
    /// 9 preset slots (index 0–8). Preset 0 = flat.
    ///
    /// Sequence:
    ///   1. GET  cls=0xe1 id=0x01 args=[0x01, 0x00]
    ///   2. SET  cls=0x95 id=0x0b args=[preset_idx, b0..b9, 0x00]  (12 bytes)
    ///   3. META cls=0xe0 id=0x06 args=[preset_idx, ...]            (7 bytes)
    ///   4. APPLY cls=0xe1 id=0x01 args=[0x02, 0x00]
    ///   5. COMMIT cls=0xeb id=0x0b args=[preset_idx, ...]          (12 bytes)
    pub const EQ_STATE_CLASS: u8 = 0xe1;
    pub const EQ_STATE_ID: u8 = 0x01;
    pub const EQ_BANDS_CLASS: u8 = 0x95;
    pub const EQ_BANDS_ID: u8 = 0x0b;
    pub const EQ_META_CLASS: u8 = 0xe0;
    pub const EQ_META_ID: u8 = 0x06;
    pub const EQ_COMMIT_CLASS: u8 = 0xeb;
    pub const EQ_COMMIT_ID: u8 = 0x0b;
    pub const EQ_PRESET_COUNT: u8 = 9;

    /// Battery level query (confirmed from startup pcap).
    ///
    /// GET: class=0x21, id=0x00, args=[0x00]
    /// Response args[0] = battery percentage (0–100 direct).
    /// Response args[1] = charging flag (0x00 = not charging).
    pub const BATTERY_CLASS: u8 = 0x21;
    pub const BATTERY_ID: u8 = 0x00;

    /// Read current sidetone level (startup/status read, not the slider SET path).
    /// Response args[0] = current level (0–15).
    pub const SIDETONE_READ_CLASS: u8 = 0x2c;

    /// THX Spatial Audio toggle (confirmed from pcap).
    ///
    /// SET: class=0xdf, id=0x01, args=[mode, 0x00]
    /// mode: 0x00 = Stereo, 0x01 = THX Spatial Audio
    pub const THX_CLASS: u8 = 0xdf;
    pub const THX_ID: u8 = 0x01;
    pub const THX_STEREO: u8 = 0x00;
    pub const THX_SPATIAL: u8 = 0x01;

    /// Active Noise Cancellation toggle + level (confirmed from pcap).
    ///
    /// SET: class=0x92, id=0x02, args=[enabled, level, 0x00]
    /// enabled: 0x00 = off, 0x01 = on
    /// level: 0x01–0x04
    pub const ANC_CLASS: u8 = 0x92;
    pub const ANC_ID: u8 = 0x02;
    pub const ANC_LEVEL_MIN: u8 = 1;
    pub const ANC_LEVEL_MAX: u8 = 4;

    /// Power savings / auto-shutoff timeout (confirmed from pcap).
    ///
    /// SET: class=0xac, id=0x01, args=[minutes, 0x00]
    /// minutes: 0x00 = disabled, 0x0f = 15 min, 0x1e = 30 min,
    ///          0x2d = 45 min, 0x3c = 60 min
    pub const POWER_SAVINGS_CLASS: u8 = 0xac;
    pub const POWER_SAVINGS_ID: u8 = 0x01;
}
