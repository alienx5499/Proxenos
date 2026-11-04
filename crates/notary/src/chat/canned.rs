//! Switch's canned texts posted under an agent's name: addressing refusals, availability
//! auto-replies and join greetings. None is a command, so the router never answers them.

/// What kind of canned text a body is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Canned {
    Refusal,
    AutoReply,
}

/// The opening words of Switch's two refusal texts (`delivery/addressing.py`).
const REFUSALS: [&str; 2] = [
    "You're not permitted to direct messages to me",
    "I only take instructions from my owner",
];

/// The opening words of the availability auto-replies (`delivery/recipient.py`).
const AUTO_REPLIES: [&str; 9] = [
    "I'm currently offline — my connector isn't reporting in.",
    "I'm not in an active session right now.",
    "Starting a session to handle this — one moment.",
    "I have a session connected to this room, but it isn't reporting as live",
    "I read room messages asynchronously, not in real time —",
    "I don't have a session connected to this room.",
    "I don't have a session in this room right now",
    "I'm not online in this room",
    "I'm stopped, so I'm not running",
];

/// The join greetings (`bridges/agent/commands.py` `AGENT_GREETINGS`), split at `{name}`.
const GREETINGS: [(&str, &str); 8] = [
    ("Hey there! Tag @", " if you need me."),
    ("Hi! Mention @", " to get my attention."),
    ("Hello! I'm ready — just tag @", " when you need me."),
    ("Hey! Ping @", " and I'll jump in."),
    ("Hi! I'm here to help — mention @", " anytime."),
    ("Hello! Tag @", " to loop me in."),
    ("Hey! I'm on standby — @", " to get started."),
    ("Hi there! Just @", " me when you're ready."),
];

/// Matches `body` after the leading `@handle` mentions (and the `— ` an owner mention is
/// followed by) that Switch puts in front of an auto-reply.
pub fn classify(body: &str) -> Option<Canned> {
    let mut rest = body.trim();
    while let Some(after) = rest.strip_prefix('@') {
        rest = after
            .split_once(char::is_whitespace)
            .map_or("", |(_, r)| r)
            .trim_start();
        rest = rest.strip_prefix("— ").unwrap_or(rest);
    }
    if REFUSALS.iter().any(|r| rest.starts_with(r)) {
        return Some(Canned::Refusal);
    }
    let stopped = rest.starts_with("you've stopped me, so I'm not running");
    if stopped || AUTO_REPLIES.iter().any(|r| rest.starts_with(r)) || is_greeting(rest) {
        return Some(Canned::AutoReply);
    }
    None
}

fn is_greeting(text: &str) -> bool {
    GREETINGS.iter().any(|(pre, post)| {
        text.strip_prefix(pre)
            .and_then(|t| t.strip_suffix(post))
            .is_some_and(|name| !name.is_empty() && !name.contains(char::is_whitespace))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_auto_reply_template_is_recognised() {
        let bodies = [
            "@notary I'm currently offline — my connector isn't reporting in. I'll respond once it reconnects.",
            "@notary I'm not in an active session right now. I'll see this when I next connect to the room.",
            "@notary Starting a session to handle this — one moment.",
            "@notary I have a session connected to this room, but it isn't reporting as live, so I'm not receiving messages — my owner may need to restart it.",
            "@notary I read room messages asynchronously, not in real time — @admin has to trigger me to pull the latest messages, either from a session I already have open or by starting a new one.",
            "@notary I don't have a session connected to this room. @admin can start one connected to **desk** by opening **x** in Switch Console.",
            "@notary I don't have a session in this room right now, but I'm working in **a**, so you can ask me there.",
            "@admin — I'm not online in this room, and @notary needs me. Open Switch Console to bring me online here.",
            "@notary I'm not online in this room. **My owner needs to open Switch Console** to bring me online here.",
            "@admin — you've stopped me, so I'm not running, and @notary needs me. Set me to running in Switch to bring me back.",
            "@notary I'm stopped, so I'm not running. **My owner has to set me to running** in Switch to bring me back.",
        ];
        for b in bodies {
            assert_eq!(classify(b), Some(Canned::AutoReply), "{b}");
        }
    }

    #[test]
    fn every_greeting_is_recognised_with_any_name() {
        for (pre, post) in GREETINGS {
            let b = format!("{pre}buyer-agent{post}");
            assert_eq!(classify(&b), Some(Canned::AutoReply), "{b}");
            assert_eq!(classify(&format!("@notary {b}")), Some(Canned::AutoReply));
            assert_eq!(classify(&format!("{pre}two words{post}")), None);
        }
    }

    #[test]
    fn refusals_and_commands_are_told_apart() {
        let r = "@notary You're not permitted to direct messages to me in this room";
        assert_eq!(classify(r), Some(Canned::Refusal));
        assert_eq!(classify("@notary offer price=1"), None);
        assert_eq!(classify("@notary I'm currently online"), None);
        assert_eq!(
            classify("Hi! Mention @notary to get my attention, please"),
            None
        );
    }
}
