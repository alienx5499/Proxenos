//! Arm D has no notary: this records offers and accepts so the run can be scored.

use serde_json::{Value, json};

use crate::prereg::Side;

#[derive(Clone, Debug)]
struct LedgerOffer {
    id: String,
    by: Side,
    terms: Value,
    note: Value,
}

/// Bookkeeping only; it never refuses or alters a message.
#[derive(Clone, Debug, Default)]
pub struct Ledger {
    offers: Vec<LedgerOffer>,
    pub agreed: Option<Value>,
    max_rounds: u32,
    kind: String,
}

impl Ledger {
    pub fn new(max_rounds: u32, kind: &str) -> Self {
        Self {
            max_rounds,
            kind: kind.to_owned(),
            ..Self::default()
        }
    }

    /// Round n holds offers 2n-1 and 2n.
    pub fn round(&self) -> u32 {
        (self.offers.len() as u32).div_ceil(2).max(1)
    }

    /// Records a Direct-grammar command sent by `side`.
    pub fn apply(&mut self, side: Side, command: &str) {
        if let Some(arg) = command.strip_prefix("offer ") {
            let Ok(v) = serde_json::from_str::<Value>(arg) else {
                return;
            };
            let id = format!("O-{}", self.offers.len() + 1);
            let note = v.get("note").cloned().unwrap_or(Value::Null);
            self.offers.push(LedgerOffer {
                id,
                by: side,
                terms: v["terms"].clone(),
                note,
            });
        } else if let Some(rest) = command.strip_prefix("accept") {
            let named = rest.trim();
            if let Some(last) = self.offers.last()
                && last.by != side
                && (named.is_empty() || named == last.id)
            {
                self.agreed = Some(last.terms.clone());
            }
        }
    }

    /// The state block `side` sees, shaped like the notary's.
    pub fn state_json(&self, side: Side) -> String {
        let offers: Vec<Value> = self
            .offers
            .iter()
            .map(|o| json!({"id": o.id, "by": o.by, "terms": o.terms, "note": o.note}))
            .collect();
        let my_turn = self.offers.last().is_none_or(|o| o.by != side);
        json!({
            "state": if self.agreed.is_some() { "Agreed" } else { "Open" },
            "kind": self.kind,
            "my_role": side,
            "round": self.round(),
            "max_rounds": self.max_rounds,
            "my_turn": my_turn,
            "offers": offers,
            "today": crate::domain_json::today().iso(),
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proxenos_domain::date::Date;

    #[test]
    fn accept_binds_only_the_counterpartys_latest_offer() {
        let mut l = Ledger::new(12, "purchase-order");
        l.apply(
            Side::Buyer,
            r#"offer {"terms":{"price_per_kg":4},"note":"a"}"#,
        );
        l.apply(Side::Buyer, "accept");
        assert!(l.agreed.is_none(), "cannot accept own offer");
        l.apply(Side::Seller, r#"offer {"terms":{"price_per_kg":5}}"#);
        assert_eq!(l.round(), 1);
        l.apply(Side::Buyer, "accept O-1");
        assert!(l.agreed.is_none(), "not the latest offer");
        assert!(l.state_json(Side::Buyer).contains("\"my_turn\":true"));
        l.apply(Side::Buyer, "accept O-2");
        assert_eq!(l.agreed, Some(json!({"price_per_kg": 5})));
        let state: Value = serde_json::from_str(&l.state_json(Side::Buyer)).unwrap();
        assert_eq!(state["today"], json!(crate::domain_json::today().iso()));
    }

    #[test]
    fn round_matches_the_domain_rule() {
        // Domain: round = (offers + 1) / 2, and round 1 before any offer.
        let mut l = Ledger::new(12, "purchase-order");
        assert_eq!(l.round(), 1);
        for (n, want) in [1, 1, 2, 2, 3, 3, 4].into_iter().enumerate() {
            let side = if n % 2 == 0 {
                Side::Buyer
            } else {
                Side::Seller
            };
            l.apply(side, r#"offer {"terms":{}}"#);
            assert_eq!(l.round(), want, "after {} offers", n + 1);
        }
    }

    #[test]
    fn lease_ledger_reports_the_lease_kind_and_records_lease_terms() {
        let mut l = Ledger::new(12, "lease");
        let terms = json!({"monthly_rent": {"minor": 180_000, "currency": "USD"},
            "deposit_months": 1, "move_in_by": Date::from_days(130).iso(), "term_months": 12});
        l.apply(Side::Seller, &format!("offer {}", json!({"terms": terms})));
        l.apply(Side::Buyer, "accept");
        assert_eq!(l.agreed, Some(terms.clone()));
        let state: Value = serde_json::from_str(&l.state_json(Side::Buyer)).unwrap();
        assert_eq!(state["kind"], json!("lease"));
        assert_eq!(state["offers"][0]["terms"], terms);
    }
}
