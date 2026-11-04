"use strict";
const token = location.pathname.split("/").filter(Boolean).pop();
const api = "/api/seal/" + encodeURIComponent(token);
const $ = (id) => document.getElementById(id);
const NAMES = {
  price_per_kg: "Price per kg", quantity_kg: "Quantity", deliver_by: "Delivery date",
  payment_net_days: "Payment terms (net days)", monthly_rent: "Monthly rent",
  deposit_months: "Deposit (months)", move_in_by: "Move-in date", term_months: "Term (months)",
};
const WORDS = {
  at_most: "At most", at_least: "At least", no_later_than: "No later than",
  no_earlier_than: "No earlier than", between: "Between",
};
let schema;

/** Shows the error window only when there is a real message; an empty message hides it. */
function fail(message) {
  $("error").textContent = message;
  $("error-win").hidden = !message;
}

function inputFor(field, part, type) {
  const id = field.field + "_" + part;
  const input = document.createElement("input");
  input.id = id;
  input.required = true;
  if (type === "date") { input.type = "date"; }
  else { input.type = "text"; input.inputMode = type === "money" ? "decimal" : "numeric"; }
  return input;
}

function render() {
  for (const c of schema.currencies) $("currency").add(new Option(c, c));
  const host = $("fields");
  for (const f of schema.fields) {
    const box = document.createElement("fieldset");
    const legend = document.createElement("legend");
    legend.textContent = (NAMES[f.field] || f.field) + (f.unit ? " (" + f.unit + ")" : "");
    box.append(legend);
    const parts = f.direction === "between" ? ["low", "high"] : ["value"];
    const row = document.createElement("div");
    row.className = "row";
    for (const part of parts) {
      const cell = document.createElement("div");
      const label = document.createElement("label");
      const input = inputFor(f, part, f.type);
      label.htmlFor = input.id;
      label.textContent = part === "value" ? WORDS[f.direction] : (part === "low" ? "From" : "To");
      cell.append(label, input);
      row.append(cell);
    }
    box.append(row);
    host.append(box);
  }
}

function value(f, part) {
  const raw = $(f.field + "_" + part).value.trim();
  const label = NAMES[f.field] || f.field;
  if (!raw) throw new Error(label + " is required.");
  if (f.type === "date") {
    const [y, m, d] = raw.split("-").map(Number);
    return Math.round(Date.UTC(y, m - 1, d) / 86400000);
  }
  if (f.type === "money") {
    if (!/^\d+(\.\d{1,2})?$/.test(raw.replace(/,/g, ""))) throw new Error(label + ": enter an amount like 22.80 or 1500.");
    const minor = Math.round(Number(raw.replace(/,/g, "")) * 100);
    return { money: { minor, currency: $("currency").value } };
  }
  if (!/^\d+$/.test(raw)) throw new Error(label + ": enter a whole number.");
  const n = Number(raw);
  return f.type === "quantity" ? { quantity: { value: n, unit: f.unit } } : { integer: n };
}

function limits() {
  const out = {};
  for (const f of schema.fields) {
    out[f.field] = f.direction === "between"
      ? { between: [value(f, "low"), value(f, "high")] }
      : { [f.direction]: value(f, "value") };
  }
  return out;
}

async function load() {
  const resp = await fetch(api);
  if (!resp.ok) { $("intro").textContent = "This seal link is unknown or was already used."; return; }
  schema = await resp.json();
  $("intro").textContent = "Deal " + schema.deal_id + " · " + schema.kind + " · you are the " + schema.side + ".";
  $("fact-deal").textContent = schema.deal_id;
  $("fact-side").textContent = schema.side;
  $("fact-kind").textContent = schema.kind;
  $("form-title").textContent = "Seal your limits: " + schema.deal_id + " " + schema.side;
  render();
  $("form-win").hidden = false;
}

let sending = false;
$("form").addEventListener("submit", async (e) => {
  e.preventDefault();
  if (sending) return;
  fail("");
  let body;
  try { body = { limits: limits() }; } catch (err) { fail(err.message); return; }
  sending = true;
  const button = $("form").querySelector("button[type=submit]");
  button.disabled = true;
  try {
    const resp = await fetch(api, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    const out = await resp.json().catch(() => ({}));
    if (!resp.ok) { fail(out.error || "The notary refused these limits."); return; }
    $("form-win").hidden = true;
    $("code").textContent = out.join_code;
    $("done").hidden = false;
    $("copy").focus();
  } catch {
    fail("Could not reach the notary. Your limits were not confirmed; try again.");
  } finally {
    sending = false;
    button.disabled = false;
  }
});

$("copy").addEventListener("click", async () => {
  try { await navigator.clipboard.writeText($("code").textContent); $("copy").textContent = "Copied"; }
  catch { $("copy").textContent = "Select and copy the code above"; }
});

load().catch(() => { $("intro").textContent = "Could not reach the notary."; });
