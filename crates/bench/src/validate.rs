//! Checks a pre-registration against the design: counts, canaries and ZOPA consistency.

use std::collections::BTreeSet;

use crate::BenchError;
use crate::prereg::{Answer, AnswerKey, Constraint, ScenarioFile, Side, number};

/// The core design: 12 scenarios, 8 with a ZOPA, 4 without, 6 with an injection, 8 in the subset.
const CORE: (usize, usize, usize, usize) = (12, 8, 6, 8);

pub fn check(scenarios: &ScenarioFile, key: &AnswerKey) -> Result<(), BenchError> {
    let bad = |msg: String| Err(BenchError::Input(msg));
    crate::domain_json::check_currency(&key.currency)?;
    let mut ids = BTreeSet::new();
    for sc in &scenarios.scenarios {
        if !ids.insert(&sc.id) {
            return bad(format!("scenario {} appears twice", sc.id));
        }
        let Some(ans) = key.scenarios.get(&sc.id) else {
            return bad(format!("scenario {} has no answer key entry", sc.id));
        };
        if sc.injection.is_some() != ans.canaries.is_some() {
            return bad(format!(
                "{}: canaries must be present exactly when there is an injection",
                sc.id
            ));
        }
        if let (Some(inj), Some(c)) = (&sc.injection, &ans.canaries)
            && (!inj.human_text.contains(&c.human) || !inj.content_note.contains(&c.content))
        {
            return bad(format!(
                "{}: the injection texts must carry their canaries",
                sc.id
            ));
        }
        zopa(&sc.id, ans)?;
    }
    if let Some(extra) = key.scenarios.keys().find(|k| !ids.contains(k)) {
        return bad(format!("answer key entry {extra} has no scenario"));
    }
    if scenarios.example || key.example {
        return Ok(());
    }
    let answers: Vec<&Answer> = scenarios
        .scenarios
        .iter()
        .map(|s| &key.scenarios[&s.id])
        .collect();
    let counts = (
        answers.len(),
        answers.iter().filter(|a| a.zopa.is_some()).count(),
        scenarios
            .scenarios
            .iter()
            .filter(|s| s.injection.is_some())
            .count(),
        answers.iter().filter(|a| a.subset8).count(),
    );
    if counts != CORE {
        return bad(format!(
            "(scenarios, with ZOPA, with injection, subset8) = {counts:?}; the pre-registered core is {CORE:?}"
        ));
    }
    Ok(())
}

/// The numeric range a constraint admits.
fn interval(c: &Constraint) -> Option<(f64, f64)> {
    Some(match c {
        Constraint::AtMost(v) | Constraint::NoLaterThan(v) => (f64::NEG_INFINITY, number(v)?),
        Constraint::AtLeast(v) | Constraint::NoEarlierThan(v) => (number(v)?, f64::INFINITY),
        Constraint::Between(lo, hi) => (number(lo)?, number(hi)?),
    })
}

/// A ZOPA exists exactly when both sides' ranges overlap on every field, and its bounds are
/// the overlap on the ZOPA field.
fn zopa(id: &str, ans: &Answer) -> Result<(), BenchError> {
    let bad = |msg: &str| Err(BenchError::Input(format!("{id}: {msg}")));
    let (Some(b), Some(s)) = (ans.limits.get(&Side::Buyer), ans.limits.get(&Side::Seller)) else {
        return bad("limits for both buyer and seller are required");
    };
    if !b.keys().eq(s.keys()) {
        return bad("buyer and seller limits name different fields");
    }
    let mut overlap = true;
    let mut zone = None;
    for (field, cb) in b {
        let (Some((l1, h1)), Some((l2, h2))) = (interval(cb), interval(&s[field])) else {
            return bad(&format!("{field}: limit values must be numbers"));
        };
        let (lo, hi) = (l1.max(l2), h1.min(h2));
        overlap &= lo <= hi;
        if ans.zopa.as_ref().is_some_and(|z| &z.field == field) {
            zone = Some((lo, hi));
        }
    }
    match (&ans.zopa, overlap) {
        (None, true) => bad("limits overlap on every field, so zopa cannot be null"),
        (Some(_), false) => bad("zopa is set but the limits do not overlap on every field"),
        (Some(z), true) if zone != Some((z.low, z.high)) => bad(&format!(
            "zopa bounds must equal the limits' overlap on {}: {zone:?}",
            z.field
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn answer(v: serde_json::Value) -> Answer {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn zopa_must_match_the_limits() {
        let limits = json!({"buyer": {"p": {"at_most": 460}}, "seller": {"p": {"at_least": 400}}});
        let ok = answer(
            json!({"zopa": {"field": "p", "low": 400, "high": 460}, "canaries": null, "limits": limits}),
        );
        assert!(zopa("S", &ok).is_ok());
        let wrong = answer(
            json!({"zopa": {"field": "p", "low": 410, "high": 460}, "canaries": null, "limits": limits}),
        );
        assert!(zopa("S", &wrong).is_err());
        let null = answer(json!({"zopa": null, "canaries": null, "limits": limits}));
        assert!(zopa("S", &null).is_err());
        let apart = json!({"buyer": {"p": {"at_most": 380}}, "seller": {"p": {"at_least": 400}}});
        assert!(
            zopa(
                "S",
                &answer(json!({"zopa": null, "canaries": null, "limits": apart}))
            )
            .is_ok()
        );
    }

    #[test]
    fn non_example_files_must_have_the_core_counts() {
        let (dir, _) = crate::prereg::tests::example_dir("counts");
        let mut sc: ScenarioFile =
            serde_json::from_slice(&std::fs::read(dir.join("scenarios.json")).unwrap()).unwrap();
        let mut key: AnswerKey =
            serde_json::from_slice(&std::fs::read(dir.join("answer_key.json")).unwrap()).unwrap();
        assert!(check(&sc, &key).is_ok());
        sc.example = false;
        key.example = false;
        assert!(
            check(&sc, &key)
                .unwrap_err()
                .to_string()
                .contains("pre-registered core")
        );
        sc.scenarios[0].injection = None;
        assert!(
            check(&sc, &key)
                .unwrap_err()
                .to_string()
                .contains("canaries")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
