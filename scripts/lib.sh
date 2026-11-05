# Shared helpers for the Tier 2 checks: Mattermost REST and the Switch Gateway via curl and jq.
# Sourced, not run. Tokens reach curl through a header file descriptor, never through argv.

# Prints the names of unset variables; succeeds only when all are set.
missing() {
  local v gaps=()
  for v in "$@"; do
    [ -n "${!v-}" ] || gaps+=("$v")
  done
  [ "${#gaps[@]}" -eq 0 ] && return 0
  echo "${gaps[*]}"
  return 1
}

for tool in curl jq; do
  command -v "$tool" >/dev/null || { echo "FAIL: $tool is required" >&2; exit 1; }
done

# mm METHOD PATH TOKEN [JSON_BODY]: one Mattermost API v4 call, body on stdout.
mm() {
  local method=$1 path=$2 token=$3 body=${4-}
  local args=(-fsS -X "$method" -H 'Content-Type: application/json' "${MM_URL%/}/api/v4$path")
  [ -n "$body" ] && args+=(--data "$body")
  curl "${args[@]}" -H @<(printf 'Authorization: Bearer %s\n' "$token")
}

# The Mattermost username behind TOKEN. Switch's sender id for that person is
# @switch-mattermost-<bridge>-<username>:localhost; an agent's is @switch-agent-<8hex>:localhost.
mm_username() { mm GET /users/me "$1" | jq -r .username; }

mm_user_id() { mm GET "/users/username/$2" "$1" | jq -r .id; }

# mm_post TOKEN CHANNEL_ID MESSAGE: prints the new post as JSON.
mm_post() {
  mm POST /posts "$1" "$(jq -nc --arg c "$2" --arg m "$3" '{channel_id: $c, message: $m}')"
}

# Replies in the thread of POST_ID by USER_ID, one JSON object per line.
mm_replies() {
  local token=$1 post=$2 user=$3
  mm GET "/posts/$post/thread" "$token" |
    jq -c --arg root "$post" --arg u "$user" \
      '.posts[] | select(.root_id == $root and .user_id == $u) | {id, message}'
}

# Root posts by USER_ID in CHANNEL_ID at or after SINCE_MS, one JSON object per line.
mm_root_posts() {
  local token=$1 channel=$2 user=$3 since=$4
  mm GET "/channels/$channel/posts?since=$since" "$token" |
    jq -c --arg u "$user" --argjson t "$since" \
      '.posts[] | select(.user_id == $u and .root_id == "" and .create_at >= $t) | {id, message}'
}

# gw METHOD PATH [JSON_BODY]: one Switch Gateway call as the operator, body on stdout.
gw() {
  local method=$1 path=$2 body=${3-}
  local args=(-fsS -X "$method" -H 'Content-Type: application/json' "${SWITCH_GATEWAY_URL%/}/gateway$path")
  [ -n "$body" ] && args+=(--data "$body")
  curl "${args[@]}" -H @<(printf 'Authorization: Bearer %s\nCookie: switch_auth=%s\n' \
    "$SWITCH_GATEWAY_KEY" "$SWITCH_GATEWAY_KEY")
}

# The agent's addressing policy as JSON (null when open).
gw_policy() { gw GET "/agents/$1" | jq -c '.addressing_policy'; }

# poll SECS CMD...: reruns CMD until it prints something or SECS pass; prints its output.
poll() {
  local secs=$1 out
  shift
  local end=$((SECONDS + secs))
  while [ "$SECONDS" -lt "$end" ]; do
    out=$("$@") || return 1
    if [ -n "$out" ]; then
      echo "$out"
      return 0
    fi
    sleep 2
  done
  return 0
}
