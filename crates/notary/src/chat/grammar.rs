//! Parses the chat command grammar. Pure.
//!
//! A command is a message that starts with `@notary` (leading whitespace allowed), then a
//! case-insensitive verb. Sides in `desk` are named by role label or letter: `buyer`,
//! `tenant` or `a` is Role A; `seller`, `landlord` or `b` is Role B. Whether the label fits
//! the deal's kind is the router's check. Terms JSON may be wrapped in backticks or a code
//! fence, and may omit `kind`: the field names pick it. Date fields take `YYYY-MM-DD`
//! (a bare day count since the Unix epoch is still read).

use proxenos_domain::deal_id::DealId;
use proxenos_domain::kind::DealKind;
use proxenos_domain::offer::{Note, OfferId};
use proxenos_domain::party::Role;
use proxenos_domain::terms::Terms;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Open {
        kind: DealKind,
        deal_id: DealId,
        /// `opener <side>` after the deal id; the default opener otherwise.
        opener: Option<SideLabel>,
    },
    Desk {
        side: SideLabel,
        deal_id: DealId,
    },
    Join {
        code: String,
    },
    /// Eight lowercase hex characters.
    Sign {
        hash8: String,
    },
    Status,
    Withdraw,
    State,
    Kinds,
    Probe {
        terms: Terms,
    },
    Offer {
        terms: Terms,
        note: Note,
    },
    Accept {
        offer_id: OfferId,
    },
}

/// A side as typed in a desk kickoff; `A` and `B` fit either kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideLabel {
    Buyer,
    Seller,
    Tenant,
    Landlord,
    A,
    B,
}

impl SideLabel {
    /// The role this label names in a deal of `kind`, or `None` when it does not fit.
    pub fn role(self, kind: DealKind) -> Option<Role> {
        use SideLabel::*;
        match (self, kind) {
            (A, _) | (Buyer, DealKind::PurchaseOrder) | (Tenant, DealKind::Lease) => Some(Role::A),
            (B, _) | (Seller, DealKind::PurchaseOrder) | (Landlord, DealKind::Lease) => {
                Some(Role::B)
            }
            _ => None,
        }
    }
}

/// Only `BadJson` quotes the input (serde's message); render strips '@' from it.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    #[error("not a command: a command starts with the notary's name")]
    NotAddressed,
    #[error("no command given")]
    Empty,
    #[error("unknown command; send kinds, state, probe, offer or accept")]
    UnknownVerb,
    #[error("usage: {0}")]
    Usage(&'static str),
    #[error("unknown deal kind; use purchase-order or lease")]
    BadKind,
    #[error("unknown side; use buyer, seller, tenant, landlord, a or b")]
    BadSide,
    #[error("deal id must look like D-0007")]
    BadDealId,
    #[error("hash must be 8 hex characters")]
    BadHash,
    #[error("offer id must look like O-5")]
    BadOfferId,
    #[error("invalid JSON: {0}")]
    BadJson(String),
    #[error("terms must use the fields of one deal kind; send kinds for the schema")]
    UnknownTerms,
    #[error("dates are calendar dates written YYYY-MM-DD")]
    BadDate,
    /// Raised by the router, which knows today; the grammar has no clock.
    #[error("date is in the past")]
    PastDate,
    /// Raised by the router, which knows the mover's previous offer.
    #[error("same terms as your last offer; change something or accept")]
    RepeatedTerms,
    #[error("message is longer than 8 KiB")]
    TooLong,
}

/// Longer bodies are refused before any parsing.
pub const MAX_BODY: usize = 8 * 1024;

const PREFIX: &str = "@notary";

/// Parses a whole message body; it must start with `@notary`.
pub fn parse(body: &str) -> Result<Command, ParseError> {
    if body.len() > MAX_BODY {
        return Err(ParseError::TooLong);
    }
    let body = body.trim_start();
    let head = body.get(..PREFIX.len()).ok_or(ParseError::NotAddressed)?;
    if !head.eq_ignore_ascii_case(PREFIX) {
        return Err(ParseError::NotAddressed);
    }
    let rest = &body[PREFIX.len()..];
    let rest = rest.strip_prefix([':', ',']).unwrap_or(rest);
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return Err(ParseError::NotAddressed);
    }
    parse_command(rest)
}

/// Parses the text after `@notary`, for deliveries that carry no prefix.
pub fn parse_command(text: &str) -> Result<Command, ParseError> {
    if text.len() > MAX_BODY {
        return Err(ParseError::TooLong);
    }
    let text = text.trim();
    let (verb, args) = text
        .split_once(char::is_whitespace)
        .map_or((text, ""), |(v, a)| (v, a.trim()));
    if verb.is_empty() {
        return Err(ParseError::Empty);
    }
    match verb.to_ascii_lowercase().as_str() {
        "open" => args::open(args),
        "desk" => args::desk(args),
        "join" => one_word(args, "join <code>").map(|code| Command::Join { code: code.into() }),
        "sign" => args::sign(args),
        "accept" => {
            let id = one_word(args, "accept <offer_id>")?;
            let offer_id = id.parse().map_err(|_| ParseError::BadOfferId)?;
            Ok(Command::Accept { offer_id })
        }
        "status" => bare(args, "status", Command::Status),
        "withdraw" => bare(args, "withdraw", Command::Withdraw),
        "state" => bare(args, "state", Command::State),
        "kinds" => bare(args, "kinds", Command::Kinds),
        "probe" => args::probe(args),
        "offer" => args::offer(args),
        _ => Err(ParseError::UnknownVerb),
    }
}

fn bare(args: &str, usage: &'static str, cmd: Command) -> Result<Command, ParseError> {
    args.is_empty()
        .then_some(cmd)
        .ok_or(ParseError::Usage(usage))
}

fn one_word<'a>(args: &'a str, usage: &'static str) -> Result<&'a str, ParseError> {
    match args.split_whitespace().collect::<Vec<_>>()[..] {
        [word] => Ok(word),
        _ => Err(ParseError::Usage(usage)),
    }
}

#[path = "grammar_args.rs"]
mod args;

#[cfg(test)]
#[path = "grammar_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "grammar_date_tests.rs"]
mod date_tests;
