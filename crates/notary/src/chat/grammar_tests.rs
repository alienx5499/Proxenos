use super::*;
use proxenos_domain::date::Date;
use proxenos_domain::money::{Currency, Money};
use proxenos_domain::quantity::Quantity;
use proxenos_domain::terms::{LeaseTerms, PurchaseOrderTerms};

const PO: &str = r#"{"price_per_kg":{"minor":109000,"currency":"INR"},"quantity_kg":{"value":400,"unit":"kg"},"deliver_by":100,"payment_net_days":45}"#;
const LEASE: &str = r#"{"monthly_rent":{"minor":2800000,"currency":"INR"},"deposit_months":2,"move_in_by":40,"term_months":11}"#;

fn po() -> Terms {
    let price = Money::new(109_000, Currency::Inr);
    let t = PurchaseOrderTerms::new(price, Quantity::kg(400), Date::from_days(100), 45);
    Terms::PurchaseOrder(t.unwrap())
}

fn lease() -> Terms {
    Terms::Lease(LeaseTerms {
        monthly_rent: Money::new(2_800_000, Currency::Inr),
        deposit_months: 2,
        move_in_by: Date::from_days(40),
        term_months: 11,
    })
}

fn id(s: &str) -> DealId {
    s.parse().unwrap()
}

#[test]
fn open_ignores_rest_of_message() {
    let want = Command::Open {
        kind: DealKind::PurchaseOrder,
        deal_id: id("D-0007"),
        opener: None,
    };
    assert_eq!(
        parse("@notary open purchase-order deal D-0007"),
        Ok(want.clone())
    );
    let long = "@notary open purchase-order deal D-0007 cafe-roaster\nmore lines";
    assert_eq!(parse(long), Ok(want));
    let lease = Command::Open {
        kind: DealKind::Lease,
        deal_id: id("D-0009"),
        opener: None,
    };
    assert_eq!(parse("@notary OPEN Lease DEAL D-0009"), Ok(lease));
    assert_eq!(
        parse("@notary open barter deal D-0007"),
        Err(ParseError::BadKind)
    );
    assert_eq!(
        parse("@notary open lease D-0007"),
        Err(ParseError::Usage(
            "open <kind> deal <D-dddd> [opener <side>]"
        ))
    );
    assert_eq!(
        parse("@notary open lease deal 7"),
        Err(ParseError::BadDealId)
    );
}

#[test]
fn open_takes_an_optional_opener_and_long_bodies_are_refused() {
    let want = Command::Open {
        kind: DealKind::PurchaseOrder,
        deal_id: id("D-0008"),
        opener: Some(SideLabel::Seller),
    };
    let body = "open purchase-order deal D-0008 opener seller cafe-roaster";
    assert_eq!(parse_command(body), Ok(want));
    let bad = parse_command("open lease deal D-0009 opener nobody");
    assert_eq!(bad, Err(ParseError::BadSide));
    let long = format!("@notary state {}", " ".repeat(MAX_BODY));
    assert_eq!(parse(&long), Err(ParseError::TooLong));
    assert_eq!(parse_command(&long[8..]), Err(ParseError::TooLong));
}

#[test]
fn side_labels_fit_their_kind_and_letters_fit_both() {
    use DealKind::{Lease, PurchaseOrder};
    assert_eq!(SideLabel::Buyer.role(PurchaseOrder), Some(Role::A));
    assert_eq!(SideLabel::Landlord.role(Lease), Some(Role::B));
    assert_eq!(SideLabel::Seller.role(Lease), None);
    assert_eq!(SideLabel::Tenant.role(PurchaseOrder), None);
    for kind in [PurchaseOrder, Lease] {
        assert_eq!(SideLabel::A.role(kind), Some(Role::A));
        assert_eq!(SideLabel::B.role(kind), Some(Role::B));
    }
}

#[test]
fn desk_accepts_labels_and_letters() {
    for (word, side) in [
        ("buyer", SideLabel::Buyer),
        ("Tenant", SideLabel::Tenant),
        ("a", SideLabel::A),
        ("SELLER", SideLabel::Seller),
        ("landlord", SideLabel::Landlord),
        ("B", SideLabel::B),
    ] {
        let want = Command::Desk {
            side,
            deal_id: id("D-0007"),
        };
        assert_eq!(
            parse(&format!("@notary desk {word} of D-0007")),
            Ok(want),
            "{word}"
        );
    }
    assert_eq!(
        parse("@notary desk broker of D-0007"),
        Err(ParseError::BadSide)
    );
    assert!(matches!(
        parse("@notary desk buyer D-0007"),
        Err(ParseError::Usage(_))
    ));
    assert!(matches!(
        parse("@notary desk buyer of D-0007 x"),
        Err(ParseError::Usage(_))
    ));
}

#[test]
fn simple_verbs_and_whitespace() {
    assert_eq!(parse("@notary status"), Ok(Command::Status));
    assert_eq!(parse("  \n\t@notary   STATUS  \n"), Ok(Command::Status));
    assert_eq!(parse("@Notary withdraw"), Ok(Command::Withdraw));
    assert_eq!(parse("@notary: state"), Ok(Command::State));
    assert_eq!(parse("@notary, kinds"), Ok(Command::Kinds));
    assert_eq!(parse("@notary\nstate"), Ok(Command::State));
    assert_eq!(
        parse("@notary status please"),
        Err(ParseError::Usage("status"))
    );
    assert_eq!(parse_command("state"), Ok(Command::State));
}

#[test]
fn join_takes_one_code() {
    let want = Command::Join {
        code: "7K3-Q9".into(),
    };
    assert_eq!(parse("@notary join 7K3-Q9"), Ok(want));
    assert!(matches!(parse("@notary join"), Err(ParseError::Usage(_))));
    assert!(matches!(
        parse("@notary join 7K3 Q9"),
        Err(ParseError::Usage(_))
    ));
}

#[test]
fn sign_checks_hash_length_and_hex() {
    let want = Command::Sign {
        hash8: "3f9a1c2e".into(),
    };
    assert_eq!(parse("@notary sign 3f9a1c2e"), Ok(want.clone()));
    assert_eq!(parse("@notary sign 3F9A1C2E"), Ok(want));
    for bad in ["3f9a1c2", "3f9a1c2e0", "3f9a1c2g", ""] {
        let got = parse(&format!("@notary sign {bad}"));
        assert!(
            matches!(got, Err(ParseError::BadHash | ParseError::Usage(_))),
            "{bad}"
        );
    }
}

#[test]
fn accept_parses_offer_id() {
    let offer_id = "O-5".parse().unwrap();
    assert_eq!(
        parse("@notary accept O-5"),
        Ok(Command::Accept { offer_id })
    );
    assert_eq!(parse("@notary accept 5"), Err(ParseError::BadOfferId));
    assert!(matches!(parse("@notary accept"), Err(ParseError::Usage(_))));
}

#[test]
fn probe_infers_kind_or_uses_tag() {
    assert_eq!(
        parse(&format!("@notary probe {PO}")),
        Ok(Command::Probe { terms: po() })
    );
    assert_eq!(
        parse(&format!("@notary probe {LEASE}")),
        Ok(Command::Probe { terms: lease() })
    );
    let tagged = PO.replacen('{', r#"{"kind":"purchase-order","#, 1);
    assert_eq!(
        parse(&format!("@notary probe {tagged}")),
        Ok(Command::Probe { terms: po() })
    );
    let wrapped = format!(r#"@notary probe {{"terms":{PO}}}"#);
    assert_eq!(parse(&wrapped), Ok(Command::Probe { terms: po() }));
}

#[test]
fn json_inside_markdown_is_unwrapped() {
    let want = Ok(Command::Probe { terms: po() });
    assert_eq!(parse(&format!("@notary probe `{PO}`")), want);
    assert_eq!(parse(&format!("@notary probe ```{PO}```")), want);
    assert_eq!(parse(&format!("@notary probe\n```json\n{PO}\n```\n")), want);
    assert_eq!(parse(&format!("@notary probe ```\n{PO}\n```")), want);
}

#[test]
fn offer_reads_terms_and_note() {
    let body = format!(r#"@notary offer {{"terms":{PO},"note":"volume matters, ping @seller"}}"#);
    let Ok(Command::Offer { terms, note }) = parse(&body) else {
        panic!("offer did not parse");
    };
    assert_eq!(terms, po());
    assert_eq!(note.as_str(), "volume matters, ping seller");
    let no_note = format!(r#"@notary offer {{"terms":{LEASE}}}"#);
    let want = Command::Offer {
        terms: lease(),
        note: Note::default(),
    };
    assert_eq!(parse(&no_note), Ok(want));
    let long = format!(
        r#"@notary offer {{"terms":{PO},"note":"{}"}}"#,
        "x".repeat(281)
    );
    assert!(matches!(parse(&long), Err(ParseError::BadJson(_))));
    let extra = format!(r#"@notary offer {{"terms":{PO},"limit":1}}"#);
    assert!(matches!(parse(&extra), Err(ParseError::BadJson(_))));
}

#[test]
fn malformed_terms_are_rejected() {
    let cases = [
        "@notary probe {",
        "@notary probe {\"price_per_kg\":1}",
        "@notary probe [1,2]",
        "@notary probe {\"color\":\"red\"}",
        "@notary probe",
        "@notary offer {\"note\":\"hi\"}",
    ];
    for c in cases {
        assert!(parse(c).is_err(), "{c}");
    }
    let mixed = PO.replacen('{', r#"{"term_months":11,"#, 1);
    assert!(matches!(
        parse(&format!("@notary probe {mixed}")),
        Err(ParseError::BadJson(_))
    ));
    assert_eq!(
        parse("@notary probe {\"x\":1}"),
        Err(ParseError::UnknownTerms)
    );
}

#[test]
fn must_start_with_notary() {
    for body in [
        "status",
        "",
        "   ",
        "hello @notary status",
        "please\n@notary status",
        "`@notary status`",
        "@notaryx status",
        "@notar status",
        "@seller-agent status",
    ] {
        assert_eq!(parse(body), Err(ParseError::NotAddressed), "{body:?}");
    }
}

#[test]
fn empty_and_unknown_verbs() {
    assert_eq!(parse("@notary"), Err(ParseError::Empty));
    assert_eq!(parse("@notary   "), Err(ParseError::Empty));
    assert_eq!(parse("@notary dance"), Err(ParseError::UnknownVerb));
    assert_eq!(parse("@notary 1240"), Err(ParseError::UnknownVerb));
}
