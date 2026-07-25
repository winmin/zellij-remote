#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
connector="$project_dir/bin/zellij-ssh-connector"
tmp_dir=${TMPDIR:-/tmp}/zellij-ssh-connector-test-$$
trap 'rm -rf "$tmp_dir"' EXIT INT TERM
mkdir -p "$tmp_dir/bin"

cat >"$tmp_dir/bin/ssh" <<'FAKE_SSH'
#!/bin/sh
{
    echo CALL
    for argument in "$@"; do
        printf '<%s>\n' "$argument"
    done
} >>"$SSH_LOG"
count=0
[ ! -f "$SSH_COUNT" ] || count=$(cat "$SSH_COUNT")
count=$((count + 1))
printf '%s\n' "$count" >"$SSH_COUNT"
status=$(printf '%s\n' "$SSH_STATUSES" | awk -v n="$count" '{ if (n <= NF) print $n; else print $NF }')
case "$status" in
    INT) kill -INT "$PPID"; exit 255 ;;
    TERM) kill -TERM "$PPID"; exit 255 ;;
    *) exit "$status" ;;
esac
FAKE_SSH

cat >"$tmp_dir/bin/sleep" <<'FAKE_SLEEP'
#!/bin/sh
printf '%s\n' "$1" >>"$SLEEP_LOG"
FAKE_SLEEP
chmod +x "$tmp_dir/bin/ssh" "$tmp_dir/bin/sleep"

export PATH="$tmp_dir/bin:$PATH"
export SSH_LOG="$tmp_dir/ssh.log"
export SSH_COUNT="$tmp_dir/ssh.count"
export SLEEP_LOG="$tmp_dir/sleep.log"

reset_fakes() {
    : >"$SSH_LOG"
    : >"$SLEEP_LOG"
    rm -f "$SSH_COUNT"
}

fail() {
    echo "connector test failed: $*" >&2
    exit 1
}

assert_status() {
    expected=$1
    shift
    set +e
    "$@"
    actual=$?
    set -e
    [ "$actual" -eq "$expected" ] || fail "expected status $expected, got $actual"
}

reset_fakes
export SSH_STATUSES="0"
"$connector" --host shell-prod --backend shell
[ "$(cat "$SSH_COUNT")" = "1" ] || fail "shell without session should run once"
[ ! -s "$SLEEP_LOG" ] || fail "successful shell should not sleep"
printf '%s\n' '<-tt>' '<-->' '<shell-prod>' >"$tmp_dir/expected.args"
awk 'NR > 1' "$SSH_LOG" >"$tmp_dir/actual.args"
cmp -s "$tmp_dir/expected.args" "$tmp_dir/actual.args" || fail "shell ssh arguments differ"

reset_fakes
export SSH_STATUSES="0"
"$connector" --host prod --backend tmux --session work
[ "$(cat "$SSH_COUNT")" = "1" ] || fail "successful ssh should run once"
[ ! -s "$SLEEP_LOG" ] || fail "successful ssh should not sleep"
printf '%s\n' '<-tt>' '<-->' '<prod>' "<exec tmux new-session -A -s 'work'>" >"$tmp_dir/expected.args"
awk 'NR > 1' "$SSH_LOG" >"$tmp_dir/actual.args"
cmp -s "$tmp_dir/expected.args" "$tmp_dir/actual.args" || fail "tmux ssh arguments differ"

reset_fakes
export SSH_STATUSES="255 255 0"
"$connector" --host dev --backend zellij --session work-2
[ "$(cat "$SSH_COUNT")" = "3" ] || fail "255 should be retried"
printf '%s\n' 1 2 >"$tmp_dir/expected.sleep"
cmp -s "$tmp_dir/expected.sleep" "$SLEEP_LOG" || fail "retry backoff differs"
grep -F "<exec zellij attach --create 'work-2'>" "$SSH_LOG" >/dev/null || fail "zellij remote command differs"

reset_fakes
export SSH_STATUSES="42"
assert_status 42 "$connector" --host prod --backend tmux --session work
[ "$(cat "$SSH_COUNT")" = "1" ] || fail "non-255 failure should not retry"
[ ! -s "$SLEEP_LOG" ] || fail "non-255 failure should not sleep"

reset_fakes
export SSH_STATUSES="INT"
assert_status 130 "$connector" --host prod --backend tmux --session work
[ ! -s "$SLEEP_LOG" ] || fail "SIGINT should stop without retrying"

reset_fakes
export SSH_STATUSES="TERM"
assert_status 143 "$connector" --host prod --backend tmux --session work
[ ! -s "$SLEEP_LOG" ] || fail "SIGTERM should stop without retrying"

reset_fakes
export SSH_STATUSES="255 255 255 255 255 255 255 0"
"$connector" --host prod --backend tmux --session work
[ "$(cat "$SSH_COUNT")" = "8" ] || fail "connector should keep retrying"
printf '%s\n' 1 2 4 8 16 16 16 >"$tmp_dir/expected.sleep"
cmp -s "$tmp_dir/expected.sleep" "$SLEEP_LOG" || fail "retry backoff should remain capped at 16 seconds"

reset_fakes
export SSH_STATUSES="0"
assert_status 2 "$connector" --host prod --backend screen --session work
assert_status 2 "$connector" --host prod --backend tmux
assert_status 2 "$connector" --host prod --backend zellij
assert_status 2 "$connector" --host prod --backend shell --session ignored
assert_status 2 "$connector" --host prod --backend shell --unrelated value
assert_status 2 "$connector" --host prod --backend tmux --session 'bad name'
assert_status 2 "$connector" --host prod --backend tmux --session 'bad;name'
[ ! -f "$SSH_COUNT" ] || fail "invalid or unrelated arguments must not invoke ssh"

echo "connector tests passed"
