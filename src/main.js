const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

/** Fixed design canvas. The whole layout is authored at this size and scaled. */
const DESIGN_W = 1500;
const DESIGN_H = 1051;

const MAX_ACCOUNTS = 5;

/**
 * The pairs an account can be sized against.
 *
 * `pipValuePerLot` is what a one-pip move on one lot is worth, in the pair's
 * quote currency. Both entries below assume the same 10,000-unit lot:
 *   EUR/USD -> 10,000 x 0.0001 =   1 USD
 *   USD/JPY -> 10,000 x 0.01   = 100 JPY
 * Change these if your broker's lot convention differs.
 */
const INSTRUMENTS = {
  "EUR/USD": { base: "EUR", quote: "USD", pipValuePerLot: 1 },
  "USD/JPY": { base: "USD", quote: "JPY", pipValuePerLot: 100 },
};

const DEFAULT_INSTRUMENT = "EUR/USD";

/**
 * Table geometry per column count (columns = 1 stop column + N accounts).
 * Widths are in design-canvas pixels; `fontScale` multiplies every cell font
 * size so five accounts still fit without reflowing.
 */
const LAYOUT = {
  2: { groupW: 400, gap: 200, fontScale: 1.15 },
  3: { groupW: 540, gap: 140, fontScale: 1.08 },
  4: { groupW: 660, gap: 100, fontScale: 1.0 },
  5: { groupW: 690, gap: 60, fontScale: 0.86 },
  6: { groupW: 700, gap: 40, fontScale: 0.74 },
};

let config = null;

// ---------- Scaling ----------

function updateScale() {
  const canvas = document.getElementById("canvas");
  const scale = Math.min(window.innerWidth / DESIGN_W, window.innerHeight / DESIGN_H);
  const left = (window.innerWidth - DESIGN_W * scale) / 2;
  const top = (window.innerHeight - DESIGN_H * scale) / 2;
  canvas.style.transform = `translate(${left}px, ${top}px) scale(${scale})`;
}

// ---------- Currency conversion ----------

/**
 * Convert between EUR / USD / JPY using the two cached rates, bridging via USD.
 * Unknown currencies pass through untouched rather than producing NaN.
 */
function toUsd(amount, currency, rates) {
  if (currency === "EUR") return amount * rates.eurusd.rate;
  if (currency === "JPY") return amount / rates.usdjpy.rate;
  return amount;
}

function fromUsd(amount, currency, rates) {
  if (currency === "EUR") return amount / rates.eurusd.rate;
  if (currency === "JPY") return amount * rates.usdjpy.rate;
  return amount;
}

function convert(amount, from, to, rates) {
  if (from === to) return amount;
  return fromUsd(toUsd(amount, from, rates), to, rates);
}

// ---------- Calculations ----------

function stopValues(range) {
  const values = [];
  for (let s = range.min; s <= range.max; s += range.step) {
    values.push(s);
  }
  return values;
}

function splitStops(values) {
  const cut = Math.ceil(values.length / 2);
  return [values.slice(0, cut), values.slice(cut)];
}

function instrumentOf(account) {
  return INSTRUMENTS[account.instrument] ?? INSTRUMENTS[DEFAULT_INSTRUMENT];
}

/**
 * Position size for one account at one stop distance.
 *
 * `contracts`: risk converted into the pair's base currency, divided by the
 * stop, truncated to a whole number.
 * `lots`: risk converted into the pair's quote currency, divided by what the
 * stop is worth on one lot, rounded to 2dp and capped at `max_lot`.
 */
function computeAccountValue(account, stop, riskFraction, rates) {
  const inst = instrumentOf(account);
  const risk = account.amount * riskFraction;

  if (account.type === "lots") {
    const riskInQuote = convert(risk, account.currency, inst.quote, rates);
    let raw = Math.round((riskInQuote / (stop * inst.pipValuePerLot)) * 100) / 100;
    if (account.max_lot != null) raw = Math.min(raw, account.max_lot);
    return raw.toFixed(2);
  }

  const riskInBase = convert(risk, account.currency, inst.base, rates);
  return String(Math.trunc(riskInBase / stop));
}

// ---------- Rendering ----------

function makeCell(className, text) {
  const div = document.createElement("div");
  div.className = `cell ${className}`;
  div.textContent = text;
  return div;
}

/** Accounts are identified by position only: #01, #02, ... */
function accountTag(index) {
  return `#${String(index + 1).padStart(2, "0")}`;
}

function makeAccountHeaderCell(index) {
  return makeCell("header-cell header-account", accountTag(index));
}

function makeStopSizeHeaderCell() {
  const div = document.createElement("div");
  div.className = "cell header-cell header-stopsize";
  ["STOP", "SIZE"].forEach((word) => {
    const span = document.createElement("span");
    span.textContent = word;
    div.appendChild(span);
  });
  return div;
}

function renderGroup(container, stops, cfg, rates) {
  const accounts = cfg.accounts.slice(0, MAX_ACCOUNTS);
  container.innerHTML = "";

  container.appendChild(makeStopSizeHeaderCell());
  accounts.forEach((_, i) => container.appendChild(makeAccountHeaderCell(i)));

  const riskFraction = cfg.risk_pct / 100;

  stops.forEach((stop) => {
    // Multiples of 10 are accented on the stop digit only, as a scale marker.
    const marked = stop % 10 === 0 ? "stop-cell row-highlight" : "stop-cell";
    container.appendChild(makeCell(marked, String(stop)));

    accounts.forEach((acc) => {
      container.appendChild(makeCell("value-cell", computeAccountValue(acc, stop, riskFraction, rates)));
    });
  });
}

/** Push the column count, geometry and font scale into CSS custom properties. */
function applyLayout(cols) {
  const layout = LAYOUT[cols] ?? LAYOUT[4];
  const left = (DESIGN_W - 2 * layout.groupW - layout.gap) / 2;
  const root = document.documentElement.style;
  root.setProperty("--cols", String(cols));
  root.setProperty("--group-w", `${layout.groupW}px`);
  root.setProperty("--group-left", `${left}px`);
  root.setProperty("--group-right", `${left + layout.groupW + layout.gap}px`);
  root.setProperty("--font-scale", String(layout.fontScale));
}

/** Centred reminder of the one input that applies to the whole table. */
function renderStatus(cfg) {
  document.getElementById("status").textContent = `RISK ${cfg.risk_pct}%`;
}

function ratesOf(cfg) {
  return { eurusd: cfg.eurusd, usdjpy: cfg.usdjpy };
}

function renderTable() {
  const accounts = config.accounts.slice(0, MAX_ACCOUNTS);
  const cols = accounts.length + 1;
  applyLayout(cols);

  const rates = ratesOf(config);
  const values = stopValues(config.stop_range);
  const [left, right] = splitStops(values);

  const leftGroup = document.getElementById("group-left");
  const rightGroup = document.getElementById("group-right");
  renderGroup(leftGroup, left, config, rates);
  renderGroup(rightGroup, right, config, rates);
  // repeat() rejects a count of 0, which an inverted stop range would produce.
  const rows = (n) => `var(--header-h) repeat(${Math.max(n, 1)}, 1fr)`;
  leftGroup.style.gridTemplateRows = rows(left.length);
  rightGroup.style.gridTemplateRows = rows(right.length);

  renderStatus(config);
}

// ---------- Settings modal ----------

function addAccountRow(acc) {
  const list = document.getElementById("accounts-list");
  if (list.children.length >= MAX_ACCOUNTS) return;

  const template = document.getElementById("account-row-template");
  const node = template.content.cloneNode(true);
  const row = node.querySelector(".account-row");
  row.querySelector(".acc-label").value = acc.label ?? "";
  row.querySelector(".acc-amount").value = acc.amount ?? 0;
  row.querySelector(".acc-currency").value = acc.currency ?? "USD";
  row.querySelector(".acc-instrument").value = acc.instrument ?? DEFAULT_INSTRUMENT;
  row.querySelector(".acc-type").value = acc.type ?? "contracts";
  row.querySelector(".acc-maxlot").value = acc.max_lot ?? "";
  row.querySelector(".acc-remove").addEventListener("click", () => {
    row.remove();
    refreshAccountRows();
  });
  list.appendChild(node);
  refreshAccountRows();
}

/** Renumber the #NN badges and keep add/remove availability honest. */
function refreshAccountRows() {
  const rows = [...document.querySelectorAll(".account-row")];
  rows.forEach((row, i) => {
    row.querySelector(".acc-tag").textContent = accountTag(i);
    // Never let the user delete their way down to an empty table.
    row.querySelector(".acc-remove").disabled = rows.length <= 1;
  });
  document.getElementById("account-add").disabled = rows.length >= MAX_ACCOUNTS;
  document.getElementById("account-count").textContent = `${rows.length} / ${MAX_ACCOUNTS}`;
}

function fillRateFields(rates) {
  document.getElementById("eurusd-rate").value = rates.eurusd.rate;
  document.getElementById("eurusd-auto").checked = rates.eurusd.auto_fetch;
  document.getElementById("eurusd-last-updated").textContent = rates.eurusd.last_updated;
  document.getElementById("usdjpy-rate").value = rates.usdjpy.rate;
  document.getElementById("usdjpy-auto").checked = rates.usdjpy.auto_fetch;
  document.getElementById("usdjpy-last-updated").textContent = rates.usdjpy.last_updated;
}

function openSettings() {
  document.getElementById("risk-input").value = config.risk_pct;
  document.getElementById("stop-min").value = config.stop_range.min;
  document.getElementById("stop-max").value = config.stop_range.max;
  document.getElementById("stop-step").value = config.stop_range.step;
  document.getElementById("always-on-top-input").checked = config.window.always_on_top;
  fillRateFields(ratesOf(config));

  document.getElementById("accounts-list").innerHTML = "";
  config.accounts.slice(0, MAX_ACCOUNTS).forEach(addAccountRow);

  document.getElementById("settings-overlay").classList.remove("hidden");
}

function closeSettings() {
  document.getElementById("settings-overlay").classList.add("hidden");
}

function collectConfigFromForm() {
  const accounts = [...document.querySelectorAll(".account-row")].map((row) => {
    const maxLotRaw = row.querySelector(".acc-maxlot").value;
    return {
      label: row.querySelector(".acc-label").value,
      amount: parseFloat(row.querySelector(".acc-amount").value) || 0,
      currency: row.querySelector(".acc-currency").value,
      instrument: row.querySelector(".acc-instrument").value,
      type: row.querySelector(".acc-type").value,
      max_lot: maxLotRaw === "" ? null : parseFloat(maxLotRaw),
    };
  });

  const readRate = (id, fallback) => parseFloat(document.getElementById(id).value) || fallback;

  return {
    accounts,
    risk_pct: parseFloat(document.getElementById("risk-input").value) || 0,
    stop_range: {
      min: parseInt(document.getElementById("stop-min").value, 10),
      max: parseInt(document.getElementById("stop-max").value, 10),
      step: Math.max(1, parseInt(document.getElementById("stop-step").value, 10) || 1),
    },
    eurusd: {
      rate: readRate("eurusd-rate", config.eurusd.rate),
      auto_fetch: document.getElementById("eurusd-auto").checked,
      last_updated: config.eurusd.last_updated,
    },
    usdjpy: {
      rate: readRate("usdjpy-rate", config.usdjpy.rate),
      auto_fetch: document.getElementById("usdjpy-auto").checked,
      last_updated: config.usdjpy.last_updated,
    },
    window: {
      ...config.window,
      always_on_top: document.getElementById("always-on-top-input").checked,
    },
  };
}

async function saveSettings() {
  const newConfig = collectConfigFromForm();
  await invoke("save_config", { new_config: newConfig });
  config = newConfig;
  renderTable();
  updatePinButton();
  closeSettings();
}

async function fetchNow(pair, buttonId) {
  const btn = document.getElementById(buttonId);
  btn.disabled = true;
  try {
    const rates = await invoke("fetch_now", { pair });
    config.eurusd = rates.eurusd;
    config.usdjpy = rates.usdjpy;
    fillRateFields(rates);
    renderTable();
  } finally {
    btn.disabled = false;
  }
}

// ---------- Window chrome ----------

function updatePinButton() {
  document.getElementById("pin-btn").classList.toggle("active", !!config.window.always_on_top);
}

async function togglePin() {
  const next = !config.window.always_on_top;
  await invoke("set_always_on_top", { value: next });
  config.window.always_on_top = next;
  updatePinButton();
}

function setupDrag() {
  document.getElementById("canvas").addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    if (e.target.closest(".chrome-btn")) return;
    invoke("start_drag");
  });
}

// ---------- Init ----------

async function init() {
  config = await invoke("get_config");
  renderTable();
  updatePinButton();
  updateScale();

  window.addEventListener("resize", updateScale);
  setupDrag();

  document.getElementById("settings-btn").addEventListener("click", openSettings);
  document.getElementById("settings-close").addEventListener("click", closeSettings);
  document.getElementById("settings-save").addEventListener("click", saveSettings);
  document.getElementById("pin-btn").addEventListener("click", togglePin);
  document.getElementById("close-btn").addEventListener("click", () => invoke("hide_to_tray"));

  document.getElementById("account-add").addEventListener("click", () =>
    addAccountRow({ label: "", amount: 0, currency: "USD", instrument: DEFAULT_INSTRUMENT, type: "contracts" })
  );
  document
    .getElementById("eurusd-fetch-now")
    .addEventListener("click", () => fetchNow("EURUSD", "eurusd-fetch-now"));
  document
    .getElementById("usdjpy-fetch-now")
    .addEventListener("click", () => fetchNow("USDJPY", "usdjpy-fetch-now"));

  // Emitted once at startup if the daily auto-fetch landed after first paint.
  listen("rates-updated", (event) => {
    config.eurusd = event.payload.eurusd;
    config.usdjpy = event.payload.usdjpy;
    renderTable();
    if (!document.getElementById("settings-overlay").classList.contains("hidden")) {
      fillRateFields(event.payload);
    }
  });
}

window.addEventListener("DOMContentLoaded", init);
