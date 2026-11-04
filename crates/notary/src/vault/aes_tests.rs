use std::collections::BTreeMap;

use proxenos_domain::date::Date;
use proxenos_domain::kind::{DealKind, Field};
use proxenos_domain::limit::Constraint;
use proxenos_domain::money::{Currency, Money};
use proxenos_domain::quantity::Quantity;
use proxenos_domain::terms::FieldValue;

use super::*;

fn key(byte: u8) -> VaultKey {
    VaultKey([byte; 32])
}

fn id(s: &str) -> DealId {
    s.parse().unwrap()
}

fn buyer_limits() -> LimitSet {
    use Constraint::*;
    let kg = |v| FieldValue::Quantity(Quantity::kg(v));
    let map = [
        (
            Field::PricePerKg,
            AtMost(FieldValue::Money(Money::new(115_037, Currency::Inr))),
        ),
        (Field::QuantityKg, Between(kg(311), kg(493))),
        (Field::DeliverBy, NoLaterThan(Date::from_days(617))),
        (Field::PaymentNetDays, AtLeast(FieldValue::Integer(31))),
    ];
    LimitSet::new(DealKind::PurchaseOrder, Role::A, BTreeMap::from(map)).unwrap()
}

#[test]
fn seal_open_round_trip_with_fresh_nonces() {
    let vault = AesVault::new(&key(1));
    let deal = id("D-0001");
    let a = vault.encrypt(&deal, Role::A, &buyer_limits()).unwrap();
    let b = vault.encrypt(&deal, Role::A, &buyer_limits()).unwrap();
    assert_ne!(a.nonce, b.nonce);
    assert_eq!(a.nonce.len(), 12);
    assert!(!String::from_utf8_lossy(&a.ciphertext).contains("115037"));
    assert_eq!(vault.decrypt(&deal, Role::A, &a).unwrap(), buyer_limits());
}

#[test]
fn ciphertext_is_bound_to_deal_and_side() {
    let vault = AesVault::new(&key(1));
    let sealed = vault
        .encrypt(&id("D-0001"), Role::A, &buyer_limits())
        .unwrap();
    assert_eq!(
        vault.decrypt(&id("D-0001"), Role::B, &sealed),
        Err(VaultError::Tampered)
    );
    assert_eq!(
        vault.decrypt(&id("D-0002"), Role::A, &sealed),
        Err(VaultError::Tampered)
    );
    let mut tampered = sealed.clone();
    tampered.ciphertext[0] ^= 1;
    let mut short = sealed.clone();
    short.nonce.pop();
    assert_eq!(
        vault.decrypt(&id("D-0001"), Role::A, &short),
        Err(VaultError::Tampered)
    );
    assert_eq!(
        vault.decrypt(&id("D-0001"), Role::A, &tampered),
        Err(VaultError::Tampered)
    );
}

#[test]
fn wrong_key_fails() {
    let sealed = AesVault::new(&key(1))
        .encrypt(&id("D-0001"), Role::A, &buyer_limits())
        .unwrap();
    let other = AesVault::new(&key(2));
    assert_eq!(
        other.decrypt(&id("D-0001"), Role::A, &sealed),
        Err(VaultError::Tampered)
    );
}

#[test]
fn missing_or_malformed_key_errors_without_echoing_it() {
    assert!(matches!(
        VaultKey::parse(None),
        Err(KeyError::Missing(VAULT_KEY_VAR))
    ));
    let short = "ab".repeat(16);
    let err = VaultKey::parse(Some(&short)).unwrap_err();
    assert!(matches!(err, KeyError::Malformed(VAULT_KEY_VAR)));
    assert!(!err.to_string().contains(&short));
    let good = "0f".repeat(32);
    let k = VaultKey::parse(Some(&good)).unwrap();
    assert_eq!(format!("{k:?}"), "VaultKey(redacted)");
}
