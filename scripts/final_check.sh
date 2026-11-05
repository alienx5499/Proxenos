#!/usr/bin/env bash
# Pre-submission check. Read-only. Prints PASS, FAIL or WARN with a reason for each check.
#   scripts/final_check.sh            every check (cargo fmt, clippy and tests included)
#   scripts/final_check.sh --fast     skip cargo test
#   scripts/final_check.sh --submit   placeholders, pre-registration, results and recorded
#                                     evidence become FAIL instead of WARN
# Exit code: 0 when nothing failed, 1 otherwise.
set -u
cd "$(dirname "$0")/.."

FAST=0
SUBMIT=0
for arg in "$@"; do
  case "$arg" in
    --fast) FAST=1 ;;
    --submit) SUBMIT=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

fails=0
warns=0
ok()   { printf '  \033[32mPASS\033[0m  %-34s %s\n' "$1" "$2"; }
bad()  { printf '  \033[31mFAIL\033[0m  %-34s %s\n' "$1" "$2"; fails=$((fails + 1)); }
warn() { printf '  \033[33mWARN\033[0m  %-34s %s\n' "$1" "$2"; warns=$((warns + 1)); }
# WARN while preparing, FAIL with --submit.
soft() { if [ "$SUBMIT" = 1 ]; then bad "$1" "$2"; else warn "$1" "$2"; fi; }
first() { head -1 | cut -c1-100; }

# Text files that would be committed: everything outside build output, git internals and
# the ignored local files.
GREP_EXCLUDES=(--exclude-dir=target --exclude-dir='target-*' --exclude-dir=.git
  --exclude=.env --exclude=.env.notary --exclude=.env.bench
  --exclude='*.db' --exclude='*.db-*' --exclude='*.key')
DOCS=(README.md docs/*.md)

echo "Proxenos final check$([ "$FAST" = 1 ] && echo ' (fast)')$([ "$SUBMIT" = 1 ] && echo ' (submit)')"

echo "-- Build"
if out=$(cargo fmt --all --check 2>&1); then ok "cargo fmt" "formatted"
else bad "cargo fmt" "$(printf '%s\n' "$out" | grep -m1 '^Diff in' || echo 'run cargo fmt --all')"; fi

if out=$(cargo clippy --workspace --all-targets --all-features --locked -- -D warnings 2>&1); then
  ok "cargo clippy -D warnings" "no warnings"
else bad "cargo clippy -D warnings" "$(printf '%s\n' "$out" | grep -m1 -E '^(error|warning)' || echo 'see cargo clippy')"; fi

if [ "$FAST" = 1 ]; then
  warn "cargo test" "skipped (--fast)"
elif out=$(cargo test --workspace --all-features --locked 2>&1); then
  passed=$(printf '%s\n' "$out" | awk '/^test result: ok/ {n += $4} END {print n + 0}')
  ok "cargo test" "$passed tests passed"
else bad "cargo test" "$(printf '%s\n' "$out" | grep -m1 -E 'FAILED|panicked|^error' || echo 'see cargo test')"; fi

echo "-- Hygiene"
years=$(grep -rnIE "${GREP_EXCLUDES[@]}" --exclude=Cargo.lock '\b(19|20)[0-9]{2}\b' . \
  | grep -vE '^\./Cargo\.toml:[0-9]+:edition = "20[0-9]{2}"$' \
  | grep -vE '^\./crates/[^:]+/Cargo\.toml:[0-9]+:.*version' \
  | grep -vE '^\./Cargo\.toml:[0-9]+:.*version')
[ -z "$years" ] && ok "No 4-digit years" "" || bad "No 4-digit years" "$(printf '%s\n' "$years" | first)"

todos=$(grep -rnE 'TODO|FIXME|unimplemented!|todo!' crates --include='*.rs' \
  | grep -vE '/tests/|tests?(_[a-z_]+)?\.rs:|_tests[a-z_]*\.rs:|/tests\.rs:')
[ -z "$todos" ] && ok "No TODO in non-test Rust" "" || bad "No TODO in non-test Rust" "$(printf '%s\n' "$todos" | first)"

# docs/ASSETS.md is the capture list and names the placeholders on purpose.
ph=$(grep -nE '<owner>|VIDEO_ID|TODO|Pending live run' "${DOCS[@]}" | grep -v '^docs/ASSETS.md:')
if [ -z "$ph" ]; then ok "No placeholders in docs" ""
else soft "No placeholders in docs" "$(printf '%s\n' "$ph" | wc -l | tr -d ' ') lines, first: $(printf '%s\n' "$ph" | first)"; fi

stray=$(grep -rniIE "${GREP_EXCLUDES[@]}" --exclude=final_check.sh "pony""tail|cave""man" .)
[ -z "$stray" ] && ok "No stray internal words" "" || bad "No stray internal words" "$(printf '%s\n' "$stray" | first)"

echo "-- Links"
# Relative links and images in README.md and docs/*.md. docs/recorded and bench/results
# are covered by their own checks below.
missing=""
nlinks=0
for doc in "${DOCS[@]}"; do
  base=$(dirname "$doc")
  targets=$( { grep -oE '\]\([^)[:space:]]+\)' "$doc" | sed -E 's/^\]\(//; s/\)$//'
               grep -oE 'src="[^"]+"' "$doc" | sed -E 's/^src="//; s/"$//'; } )
  for t in $targets; do
    case "$t" in http://*|https://*|mailto:*|\#*) continue ;; esac
    t=${t%%#*}
    case "$t" in docs/recorded/*|bench/results/*|../bench/results/*|recorded/*) continue ;; esac
    nlinks=$((nlinks + 1))
    [ -e "$base/$t" ] || missing="$missing $doc:$t"
  done
done
if [ -z "$missing" ]; then ok "Relative links resolve" "$nlinks checked"
else bad "Relative links resolve" "$(echo $missing | wc -w | tr -d ' ') missing:$(echo $missing | cut -c1-80)"; fi

[ -s LICENSE ] && ok "LICENSE" "$(head -1 LICENSE)" || bad "LICENSE" "missing"

echo "-- Secrets"
leaks=""
for f in .env*.example; do
  [ -e "$f" ] || continue
  hit=$(grep -nE '^[A-Z0-9_]+=.*([0-9a-fA-F]{64}|sk-[A-Za-z0-9_-]{16,}|ghp_[A-Za-z0-9]{20,}|[A-Za-z0-9+/_-]{40,})' "$f")
  [ -n "$hit" ] && leaks="$leaks $f:$(printf '%s\n' "$hit" | cut -d= -f1 | head -1)"
done
[ -z "$leaks" ] && ok "Example env files hold no secrets" "$(ls .env*.example 2>/dev/null | wc -l | tr -d ' ') files" \
  || bad "Example env files hold no secrets" "real-looking value in$leaks"

# Local secret files may exist only when .gitignore covers them.
unignored=""
present=""
while IFS= read -r f; do
  [ -n "$f" ] || continue
  name=$(basename "$f")
  present="$present $f"
  covered=0
  while IFS= read -r pat; do
    pat=${pat#/}
    case "$pat" in ''|\#*) continue ;; esac
    # shellcheck disable=SC2254
    case "$name" in $pat) covered=1; break ;; esac
  done < .gitignore
  [ "$covered" = 1 ] || unignored="$unignored $f"
done < <(find . \( -path ./target -o -path './target-*' -o -path ./.git \) -prune -o -type f \
  \( -name '.env' -o -name '.env.*' -o -name '*.db' -o -name '*.db-*' -o -name '*.key' \) \
  ! -name '*.example' -print)
if [ -n "$unignored" ]; then bad "No unignored secret files" "not in .gitignore:$unignored"
elif [ -n "$present" ]; then ok "No unignored secret files" "local only, ignored:$present"
else ok "No unignored secret files" "none present"; fi

echo "-- Evidence"
pre=bench/prereg
pmiss=""
for f in answer_key.json answer_key.sha256 scenarios.json scenarios.sha256; do
  [ -s "$pre/$f" ] || pmiss="$pmiss $f"
done
if [ -n "$pmiss" ]; then soft "Pre-registration files" "missing in $pre:$pmiss"
else
  bad_hash=""
  for n in answer_key scenarios; do
    want=$(awk '{print $1; exit}' "$pre/$n.sha256")
    got=$(shasum -a 256 "$pre/$n.json" | awk '{print $1}')
    [ "$want" = "$got" ] || bad_hash="$bad_hash $n"
  done
  [ -z "$bad_hash" ] && ok "Pre-registration files" "hashes match" || soft "Pre-registration files" "hash mismatch:$bad_hash"
fi

[ -s bench/results/RESULTS.md ] && ok "bench/results/RESULTS.md" "present" \
  || soft "bench/results/RESULTS.md" "missing (run the bench, then --rescore)"

rmiss=""
nrec=0
for r in $(grep -ohE 'docs/recorded/[A-Za-z0-9._-]+' "${DOCS[@]}" | sort -u); do
  nrec=$((nrec + 1))
  [ -e "$r" ] || rmiss="$rmiss ${r#docs/recorded/}"
done
[ -z "$rmiss" ] && ok "Recorded evidence files" "$nrec referenced, all present" \
  || soft "Recorded evidence files" "missing in docs/recorded:$rmiss"

echo "-- README"
grep -qE '^## .*Switch' README.md && ok "README has a Switch section" "$(grep -m1 -E '^## .*Switch' README.md)" \
  || bad "README has a Switch section" "no '## ... Switch' heading"
grep -q 'Track 01' README.md && grep -q 'Track 02' README.md && ok "README names both tracks" "Track 01 and Track 02" \
  || bad "README names both tracks" "Track 01 or Track 02 missing"

echo
if [ "$fails" = 0 ]; then echo "READY: no failures ($warns warnings)."
else echo "NOT READY: $fails failed, $warns warnings."; fi
[ "$fails" = 0 ]
