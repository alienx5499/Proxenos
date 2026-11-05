#!/usr/bin/env bash
# Tier 2: the notary's party rule. A non-party's `@notary sign` is refused by Switch and never
# reaches the notary; a party's `@notary status` is answered. With CHECK_REMOVE_RULE=1 the
# notary's policy is opened for one try, the same line reaches the notary and its own check
# answers `not a party to <deal>`; the saved policy is then put back.
#
# Posts through Mattermost REST as two accounts (live check L11).
# Required: MM_URL, MM_DEAL_CHANNEL_ID, DEAL_ID, MM_NONPARTY_TOKEN (personal access token of a
# deal-room member who is not a party, e.g. the look-alike), MM_PARTY_TOKEN (a bound owner,
# one of the Mattermost usernames given to `proxenos admit --owner`).
# Optional: NOTARY_NAME (default notary), MM_NOTARY_USERNAME (the notary's Mattermost
# username, default NOTARY_NAME), HASH8
# (default 00000000), CHECK_WAIT_SECS (default 60), CHECK_QUIET_SECS (default 20).
# CHECK_REMOVE_RULE=1 also needs SWITCH_GATEWAY_URL, SWITCH_GATEWAY_KEY (operator) and
# SWITCH_AGENT_ID (the notary). Exit: 0 PASS, 1 FAIL, 2 not run (manual steps printed).
set -euo pipefail
. "$(dirname "$0")/lib.sh"

NOTARY_NAME=${NOTARY_NAME:-notary}
MM_NOTARY_USERNAME=${MM_NOTARY_USERNAME:-$NOTARY_NAME}
HASH8=${HASH8:-00000000}
WAIT=${CHECK_WAIT_SECS:-60}
QUIET=${CHECK_QUIET_SECS:-20}
REFUSAL="not permitted to direct messages to me"
SIGN="@$NOTARY_NAME sign $HASH8"

need=(MM_URL MM_DEAL_CHANNEL_ID DEAL_ID MM_NONPARTY_TOKEN MM_PARTY_TOKEN)
[ "${CHECK_REMOVE_RULE-}" = 1 ] && need+=(SWITCH_GATEWAY_URL SWITCH_GATEWAY_KEY SWITCH_AGENT_ID)
if ! gaps=$(missing "${need[@]}"); then
  cat <<EOF
NOT RUN: $gaps not set. Manual steps (after \`proxenos admit <deal>\`):
  1. As a deal-room member who is not a party, type in the deal room: $SIGN
     PASS if Switch posts "You're not permitted to direct messages to me ..." in that
     message's thread as @$NOTARY_NAME, and the notary posts nothing at the room root.
  2. As a bound owner, type in the deal room: @$NOTARY_NAME status
     PASS if the notary answers with the status line, starting with the deal id.
  3. Optional: as the operator, PUT {"policy": null} on the notary, repeat step 1 and expect
     the notary's own "not a party to <deal>"; then put the saved policy back.
EOF
  exit 2
fi

fail() {
  echo "FAIL: $*"
  exit 1
}

bot=$(mm_user_id "$MM_PARTY_TOKEN" "$MM_NOTARY_USERNAME")
nonparty=$(mm_username "$MM_NONPARTY_TOKEN")
party=$(mm_username "$MM_PARTY_TOKEN")
if missing SWITCH_GATEWAY_URL SWITCH_GATEWAY_KEY SWITCH_AGENT_ID >/dev/null; then
  echo "notary policy: $(gw_policy "$SWITCH_AGENT_ID")"
fi

# post_and_watch TOKEN MESSAGE EXPECTED: posts, then sets post_id, replies (thread replies by
# the notary's bot) and roots (its root posts). EXPECTED (replies or roots) is awaited up to
# CHECK_WAIT_SECS and stops at the first hit; the other is watched for CHECK_QUIET_SECS.
post_and_watch() {
  local post since thread_wait=$QUIET root_wait=$WAIT
  [ "$3" = replies ] && thread_wait=$WAIT root_wait=$QUIET
  post=$(mm_post "$1" "$MM_DEAL_CHANNEL_ID" "$2")
  post_id=$(jq -r .id <<<"$post")
  since=$(jq -r .create_at <<<"$post")
  replies=$(poll "$thread_wait" mm_replies "$1" "$post_id" "$bot")
  roots=$(poll "$root_wait" mm_root_posts "$1" "$MM_DEAL_CHANNEL_ID" "$bot" "$since")
  echo "  posted $post_id: $2"
  echo "  thread replies by @$MM_NOTARY_USERNAME: ${replies:-none}"
  echo "  root posts by @$MM_NOTARY_USERNAME since: ${roots:-none}"
}

echo "1. non-party (Mattermost username $nonparty) addresses the notary"
post_and_watch "$MM_NONPARTY_TOKEN" "$SIGN" replies
grep -qF "$REFUSAL" <<<"$replies" || fail "Switch posted no refusal for the non-party"
[ -z "$roots" ] || fail "the notary answered the non-party, so the line reached it"

echo "2. party (Mattermost username $party) addresses the notary"
post_and_watch "$MM_PARTY_TOKEN" "@$NOTARY_NAME status" roots
grep -qF "$REFUSAL" <<<"$replies" && fail "Switch refused the party"
grep -qF "$DEAL_ID · " <<<"$roots" || fail "no status line for $DEAL_ID from the notary"

if [ "${CHECK_REMOVE_RULE-}" = 1 ]; then
  saved=$(gw_policy "$SWITCH_AGENT_ID")
  restore() {
    gw PUT "/agents/$SWITCH_AGENT_ID/addressing-policy" \
      "$(jq -c '{policy: .}' <<<"$saved")" >/dev/null
    [ "$(gw_policy "$SWITCH_AGENT_ID")" = "$saved" ] ||
      echo "WARNING: the notary policy differs from the saved one: $saved" >&2
  }
  trap restore EXIT
  echo "3. notary policy opened; the non-party tries again"
  gw PUT "/agents/$SWITCH_AGENT_ID/addressing-policy" '{"policy": null}' >/dev/null
  post_and_watch "$MM_NONPARTY_TOKEN" "$SIGN" roots
  grep -qF "not a party to $DEAL_ID" <<<"$roots" || fail "the notary did not refuse the non-party itself"
  echo "with the policy open, the notary's own check refused the non-party"
fi

echo "PASS: Switch refused the non-party before the notary; the party was admitted"
