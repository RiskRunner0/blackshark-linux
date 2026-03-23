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
        // data_size counts the class byte, sub byte, command ID byte, and all arg bytes.
        assert!(args.len() <= 49, "argument data exceeds report capacity");

        let mut buf = [0u8; REPORT_LEN];
        buf[0] = 0x02; // report ID
        buf[1] = 0x00; // status: new command
        buf[2] = transaction_id;
        // buf[3..8] = 0x00
        buf[9]  = 0x80; // flags (constant, observed in all Synapse captures)
        buf[10] = class;
        buf[11] = 0x00; // sub (always 0 in captures)
        buf[12] = id;
        let data_size = 3 + args.len(); // class + sub + id + args
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

    /// EQ preset activation — 5-command sequence per preset switch.
    /// Preset index 0x00–0x04 in args[0].
    /// TODO: document full EQ band encoding once implemented.
    pub const EQ_STATE_CLASS_GET: u8 = 0xe1;
    pub const EQ_STATE_CLASS_SET: u8 = 0xe1;
    pub const EQ_STATE_ID: u8 = 0x01;
    pub const EQ_BANDS_CLASS: u8 = 0x95;
    pub const EQ_BANDS_ID: u8 = 0x0b;
    pub const EQ_META_CLASS: u8 = 0xe0;
    pub const EQ_META_ID: u8 = 0x06;
    pub const EQ_COMMIT_CLASS: u8 = 0xeb;
    pub const EQ_COMMIT_ID: u8 = 0x0b;

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
}
