//! Pulls the one command out of a CLI reply.

use serde_json::Value;

/// Which verbs a reply may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grammar {
    /// Commands to the notary: state, kinds, probe, offer, accept <id>.
    Notary,
    /// Notary off (arm D): offer and accept [id], addressed to the counterparty agent.
    Direct,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// Normalised command text, without any leading `@name`.
    Command {
        text: String,
        extra: usize,
    },
    /// No command: the reply is posted as it stands.
    Text(String),
    Empty,
}

pub fn extract(output: &str, grammar: Grammar) -> Reply {
    let mut found = Vec::new();
    let mut offset = 0;
    for line in output.split_inclusive('\n') {
        if let Some(cmd) = command_at(line, &output[offset..], grammar) {
            found.push(cmd);
        }
        offset += line.len();
    }
    match found.len() {
        0 if output.trim().is_empty() => Reply::Empty,
        0 => Reply::Text(output.trim().to_owned()),
        n => Reply::Command {
            text: found.swap_remove(0),
            extra: n - 1,
        },
    }
}

/// `line` starts at `rest`; JSON arguments may run past the line end.
fn command_at(line: &str, rest: &str, grammar: Grammar) -> Option<String> {
    let body = strip_lead(line);
    let lead = line.len() - body.len();
    let verb_len = body
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(body.len());
    let (verb, after) = body.split_at(verb_len);
    let tail_start = lead + verb_len;
    match (verb, grammar) {
        ("state" | "kinds", Grammar::Notary) if bare(after) => Some(verb.to_owned()),
        ("probe", Grammar::Notary) => json_arg(&rest[tail_start..]).map(|v| format!("probe {v}")),
        ("offer", _) => json_arg(&rest[tail_start..])
            .filter(|v| v.get("terms").is_some_and(Value::is_object))
            .map(|v| format!("offer {v}")),
        ("accept", _) => {
            // The whole line must be the command: `accept O-5`, or bare `accept` in arm D.
            let arg = after.trim().trim_matches('`').trim();
            if offer_id(arg) {
                Some(format!("accept {arg}"))
            } else if arg.is_empty() && grammar == Grammar::Direct {
                Some("accept".to_owned())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Drops quoting, list markers, backticks and a leading `@name` mention.
fn strip_lead(line: &str) -> &str {
    let mut s = line
        .trim_start()
        .trim_start_matches(['>', '-', '*', '`', ' ']);
    if let Some(r) = s.strip_prefix('@') {
        s = r
            .split_once(char::is_whitespace)
            .map_or("", |(_, t)| t)
            .trim_start();
    }
    s.trim_start_matches('`')
}

/// `O-<n>`, n a positive integer without leading zeros.
fn offer_id(s: &str) -> bool {
    s.strip_prefix("O-").is_some_and(|n| {
        n.starts_with(|c: char| ('1'..='9').contains(&c)) && n.chars().all(|c| c.is_ascii_digit())
    })
}

fn bare(after: &str) -> bool {
    after.trim().trim_matches(['`', '.']).is_empty()
}

/// The first JSON object after the verb, compacted.
fn json_arg(s: &str) -> Option<Value> {
    let s = s.trim_start();
    if !s.starts_with('{') {
        return None;
    }
    let mut it = serde_json::Deserializer::from_str(s).into_iter::<Value>();
    it.next()?.ok().filter(Value::is_object)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(s: &str) -> String {
        match extract(s, Grammar::Notary) {
            Reply::Command { text, .. } => text,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn finds_commands_in_prose_and_fences() {
        assert_eq!(cmd("state"), "state");
        assert_eq!(cmd("@notary accept O-5\n"), "accept O-5");
        let multi = "I will counter.\n```\n@notary offer {\"terms\": {\"price_per_kg\": 4.2},\n \"note\": \"fair\"}\n```";
        assert_eq!(
            cmd(multi),
            r#"offer {"note":"fair","terms":{"price_per_kg":4.2}}"#
        );
        assert_eq!(
            cmd("`probe {\"quantity_kg\": 400}`"),
            r#"probe {"quantity_kg":400}"#
        );
    }

    #[test]
    fn prose_is_text_and_first_command_wins() {
        assert_eq!(
            extract("the state of play is fine", Grammar::Notary),
            Reply::Text("the state of play is fine".into())
        );
        assert_eq!(extract("  \n", Grammar::Notary), Reply::Empty);
        assert_eq!(
            extract("offer {\"note\":\"x\"}", Grammar::Notary),
            Reply::Text("offer {\"note\":\"x\"}".into())
        );
        let r = extract("state\naccept O-2", Grammar::Notary);
        assert_eq!(
            r,
            Reply::Command {
                text: "state".into(),
                extra: 1
            }
        );
    }

    #[test]
    fn direct_grammar_has_no_probe_and_bare_accept() {
        assert_eq!(
            extract("accept", Grammar::Direct),
            Reply::Command {
                text: "accept".into(),
                extra: 0
            }
        );
        assert!(matches!(
            extract("probe {}", Grammar::Direct),
            Reply::Text(_)
        ));
        assert!(matches!(extract("accept", Grammar::Notary), Reply::Text(_)));
        assert!(matches!(
            extract("accept.", Grammar::Direct),
            Reply::Text(_)
        ));
        assert!(matches!(
            extract("accept these terms", Grammar::Direct),
            Reply::Text(_)
        ));
    }

    #[test]
    fn accept_id_must_be_an_offer_id() {
        assert_eq!(cmd("accept `O-12`"), "accept O-12");
        for bad in [
            "accept O-0",
            "accept O-05",
            "accept 5",
            "accept O-5 now",
            "accept X-5",
        ] {
            assert!(
                matches!(extract(bad, Grammar::Notary), Reply::Text(_)),
                "{bad}"
            );
        }
    }
}
