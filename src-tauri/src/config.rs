use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Hard cap on how many accounts the widget will render. The table layout and
/// font-scale steps in the frontend are tuned for 1..=MAX_ACCOUNTS columns.
pub const MAX_ACCOUNTS: usize = 5;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Account {
    pub label: String,
    pub amount: f64,
    /// Currency the account balance is denominated in: "USD" | "EUR" | "JPY".
    pub currency: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub max_lot: Option<f64>,
    /// Symbol of the pair this account is sized against, e.g. "EUR/USD".
    /// `None` in a config written by an older build; filled in by `migrate`.
    #[serde(default)]
    pub instrument: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StopRange {
    pub min: i64,
    pub max: i64,
    pub step: i64,
}

/// A cached FX rate plus its refresh policy. One of these per pair.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RateState {
    pub rate: f64,
    pub auto_fetch: bool,
    pub last_updated: String,
}

impl RateState {
    fn new(rate: f64) -> Self {
        RateState {
            rate,
            auto_fetch: true,
            last_updated: "1970-01-01".into(),
        }
    }
}

fn default_usdjpy() -> RateState {
    RateState::new(150.0)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub always_on_top: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub accounts: Vec<Account>,
    pub risk_pct: f64,
    pub stop_range: StopRange,
    pub eurusd: RateState,
    #[serde(default = "default_usdjpy")]
    pub usdjpy: RateState,
    pub window: WindowState,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            accounts: vec![
                Account {
                    label: "Account 1".into(),
                    amount: 100_000.0,
                    currency: "USD".into(),
                    kind: "contracts".into(),
                    max_lot: None,
                    instrument: Some("EUR/USD".into()),
                },
                Account {
                    label: "Account 2".into(),
                    amount: 50_000.0,
                    currency: "USD".into(),
                    kind: "contracts".into(),
                    max_lot: None,
                    instrument: Some("EUR/USD".into()),
                },
                Account {
                    label: "Account 3".into(),
                    amount: 5_000_000.0,
                    currency: "JPY".into(),
                    kind: "lots".into(),
                    max_lot: Some(100.0),
                    instrument: Some("USD/JPY".into()),
                },
            ],
            risk_pct: 0.55,
            stop_range: StopRange { min: 4, max: 50, step: 2 },
            eurusd: RateState::new(1.15),
            usdjpy: RateState::new(150.0),
            window: WindowState {
                x: 100,
                y: 100,
                width: 1200,
                height: 800,
                always_on_top: true,
            },
        }
    }
}

/// Fill in anything a config written by an older build is missing, and clamp
/// values the frontend assumes are in range. Runs on every load and on save.
pub fn migrate(cfg: &mut Config) {
    if cfg.accounts.is_empty() {
        cfg.accounts = Config::default().accounts;
    }
    cfg.accounts.truncate(MAX_ACCOUNTS);
    for acc in &mut cfg.accounts {
        if acc.instrument.is_none() {
            // Pre-instrument configs sized `contracts` accounts off EUR/USD and
            // `lots` accounts off USD/JPY; keep those numbers identical.
            acc.instrument = Some(if acc.kind == "lots" { "USD/JPY" } else { "EUR/USD" }.into());
        }
    }
    if cfg.stop_range.step < 1 {
        cfg.stop_range.step = 1;
    }
}

pub fn config_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").expect("APPDATA not set");
    PathBuf::from(appdata).join("StopSizeWidget").join("config.json")
}

pub fn load_or_create() -> Config {
    let path = config_path();
    if let Ok(text) = fs::read_to_string(&path) {
        if let Ok(mut cfg) = serde_json::from_str::<Config>(&text) {
            migrate(&mut cfg);
            return cfg;
        }
    }
    let cfg = Config::default();
    let _ = save(&cfg);
    cfg
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(cfg).unwrap();
    fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config file as written by the pre-instrument build: no `usdjpy` block,
    /// no `instrument` on accounts, and a `logo` field that no longer exists.
    /// Loading one must preserve the user's balances rather than fall back to
    /// defaults, and must reproduce the old hardcoded pairings exactly.
    #[test]
    fn migrates_a_pre_instrument_config() {
        let old = r#"{
          "accounts": [
            { "label": "A", "amount": 123456, "currency": "USD", "type": "contracts",
              "max_lot": null, "logo": "assets/logos/a.png" },
            { "label": "B", "amount": 654321, "currency": "JPY", "type": "lots",
              "max_lot": 1.75, "logo": "assets/logos/b.png" }
          ],
          "risk_pct": 0.33,
          "stop_range": { "min": 4, "max": 50, "step": 2 },
          "eurusd": { "rate": 1.15, "auto_fetch": true, "last_updated": "2026-07-15" },
          "window": { "x": 1, "y": 2, "width": 1200, "height": 800, "always_on_top": true }
        }"#;

        let mut cfg: Config = serde_json::from_str(old).expect("old config must still parse");
        migrate(&mut cfg);

        assert_eq!(cfg.accounts[0].amount, 123456.0);
        assert_eq!(cfg.accounts[1].amount, 654321.0);
        assert_eq!(cfg.accounts[1].max_lot, Some(1.75));
        assert_eq!(cfg.accounts[0].instrument.as_deref(), Some("EUR/USD"));
        assert_eq!(cfg.accounts[1].instrument.as_deref(), Some("USD/JPY"));
        assert_eq!(cfg.eurusd.rate, 1.15);
        assert_eq!(cfg.usdjpy.rate, default_usdjpy().rate);
        assert_eq!(cfg.window.x, 1);
    }

    #[test]
    fn clamps_account_count_and_step() {
        let mut cfg = Config::default();
        cfg.accounts = (0..9).map(|_| cfg.accounts[0].clone()).collect();
        cfg.stop_range.step = 0;
        migrate(&mut cfg);
        assert_eq!(cfg.accounts.len(), MAX_ACCOUNTS);
        assert_eq!(cfg.stop_range.step, 1);
    }

    #[test]
    fn empty_account_list_falls_back_to_defaults() {
        let mut cfg = Config::default();
        cfg.accounts = vec![];
        migrate(&mut cfg);
        assert!(!cfg.accounts.is_empty());
    }
}
