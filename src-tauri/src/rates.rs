//! Keyless daily FX lookups. Frankfurter (ECB reference rates) is the primary
//! source, open.er-api.com the fallback. Both are hit from Rust rather than the
//! WebView so no network permissions leak into the frontend.

use reqwest::Client;
use std::ops::RangeInclusive;
use std::time::Duration;

/// The pairs the widget knows how to size against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pair {
    EurUsd,
    UsdJpy,
}

impl Pair {
    pub fn parse(s: &str) -> Option<Pair> {
        match s.to_ascii_uppercase().replace('/', "").as_str() {
            "EURUSD" => Some(Pair::EurUsd),
            "USDJPY" => Some(Pair::UsdJpy),
            _ => None,
        }
    }

    fn base(self) -> &'static str {
        match self {
            Pair::EurUsd => "EUR",
            Pair::UsdJpy => "USD",
        }
    }

    fn quote(self) -> &'static str {
        match self {
            Pair::EurUsd => "USD",
            Pair::UsdJpy => "JPY",
        }
    }

    /// Anything outside this band is treated as a bad response and discarded in
    /// favour of the cached value.
    fn sane_range(self) -> RangeInclusive<f64> {
        match self {
            Pair::EurUsd => 0.5..=2.0,
            Pair::UsdJpy => 50.0..=300.0,
        }
    }
}

pub async fn fetch(pair: Pair) -> Option<f64> {
    let client = Client::builder().timeout(Duration::from_secs(5)).build().ok()?;

    let frankfurter = format!(
        "https://api.frankfurter.app/latest?from={}&to={}",
        pair.base(),
        pair.quote()
    );
    if let Some(rate) = try_source(&client, &frankfurter, pair).await {
        return Some(rate);
    }

    let erapi = format!("https://open.er-api.com/v6/latest/{}", pair.base());
    try_source(&client, &erapi, pair).await
}

/// Both APIs answer with `{ "rates": { "<QUOTE>": <number> }, ... }`.
async fn try_source(client: &Client, url: &str, pair: Pair) -> Option<f64> {
    let resp = client.get(url).send().await.ok()?;
    let json: serde_json::Value = resp.json().await.ok()?;
    let rate = json.get("rates")?.get(pair.quote())?.as_f64()?;
    validate(rate, pair)
}

fn validate(rate: f64, pair: Pair) -> Option<f64> {
    if rate.is_finite() && pair.sane_range().contains(&rate) {
        Some(rate)
    } else {
        None
    }
}
