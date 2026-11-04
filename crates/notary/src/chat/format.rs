//! Human text for values in chat: money, calendar days, time of day, terms summaries.

use proxenos_domain::date::Date;
use proxenos_domain::money::{Currency, Money};
use proxenos_domain::terms::Terms;
use proxenos_domain::timestamp::Timestamp;

/// `$1,500` or `$22.40`; thousands grouped with commas.
pub fn money(m: Money) -> String {
    let symbol = match m.currency {
        Currency::Inr => "₹",
        Currency::Usd => "$",
    };
    let sign = if m.minor < 0 { "-" } else { "" };
    let abs = m.minor.unsigned_abs();
    let digits = (abs / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    match abs % 100 {
        0 => format!("{sign}{symbol}{grouped}"),
        cents => format!("{sign}{symbol}{grouped}.{cents:02}"),
    }
}

pub(crate) use proxenos_domain::date::civil;

/// ISO `YYYY-MM-DD` for a day count since the Unix epoch (proleptic Gregorian).
pub fn date(d: Date) -> String {
    d.iso()
}

/// `HH:MM` in UTC.
pub fn time_of_day(t: Timestamp) -> String {
    let s = t.seconds().rem_euclid(86_400);
    format!("{:02}:{:02}", s / 3_600, s % 3_600 / 60)
}

/// One line: `$22.40/kg × 400 kg, deliver by <date>, Net-45`.
pub fn terms(t: &Terms) -> String {
    match t {
        Terms::PurchaseOrder(p) => format!(
            "{}/kg × {} kg, deliver by {}, Net-{}",
            money(p.price_per_kg()),
            p.quantity_kg().value,
            date(p.deliver_by()),
            p.payment_net_days()
        ),
        Terms::Lease(l) => format!(
            "{}/month, deposit {} months, move in by {}, {}-month term",
            money(l.monthly_rent),
            l.deposit_months,
            date(l.move_in_by),
            l.term_months
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_is_iso() {
        assert_eq!(date(Date::from_days(40)), Date::from_days(40).iso());
        assert_eq!(civil(0).1, 1);
    }

    #[test]
    fn money_groups_and_shows_cents() {
        let inr = |m| money(Money::new(m, Currency::Inr));
        assert_eq!(inr(109_000), "₹1,090");
        assert_eq!(inr(2_800_000), "₹28,000");
        assert_eq!(inr(123_456_789), "₹1,234,567.89");
        assert_eq!(inr(5), "₹0.05");
        assert_eq!(inr(-100_000), "-₹1,000");
        let usd = |m| money(Money::new(m, Currency::Usd));
        assert_eq!(usd(2_240), "$22.40");
        assert_eq!(usd(150_000), "$1,500");
        assert_eq!(usd(99_900), "$999");
    }

    #[test]
    fn time_of_day_is_utc_hours_minutes() {
        assert_eq!(time_of_day(Timestamp(0)), "00:00");
        assert_eq!(time_of_day(Timestamp(18 * 3_600 + 5 * 60 + 59)), "18:05");
        assert_eq!(time_of_day(Timestamp(86_400 + 60)), "00:01");
        assert_eq!(time_of_day(Timestamp(-60)), "23:59");
    }
}
