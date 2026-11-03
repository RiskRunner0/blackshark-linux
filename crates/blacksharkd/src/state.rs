#[derive(Clone, Debug)]
pub struct SharedState {
    pub connected: bool,
    pub battery_pct: u8,
    pub charging: bool,
    pub sidetone: u8,
    pub eq_preset: u8,
    pub anc_enabled: bool,
    pub anc_level: u8,
    pub power_savings_minutes: u8,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            connected: false,
            battery_pct: 0,
            charging: false,
            sidetone: 0,
            eq_preset: 0,
            anc_enabled: false,
            anc_level: 1,
            power_savings_minutes: 0,
        }
    }
}
