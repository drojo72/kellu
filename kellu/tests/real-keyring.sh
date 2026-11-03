#!/bin/bash
# Manual end-to-end check of `kellu keys` against a REAL Secret Service (gnome-keyring), in a
# throw-away session: its own D-Bus bus, home directory and keyring. It never touches your
# real keyring and uses only made-up secrets.
#
# Needs: gnome-keyring, libsecret-tools (secret-tool), dbus (dbus-run-session), systemd's busctl.
#   Debian/Ubuntu: sudo apt install gnome-keyring libsecret-tools dbus systemd
#
# Usage: kellu/tests/real-keyring.sh /path/to/kellu      (for example target/debug/kellu)
#
# This is not part of `cargo test`: CI has no keyring. It exists because the fakes used by the
# unit tests cannot prove that the real tools behave as assumed (a first version of the listing
# passed every fake-based test and listed one entry out of four on a real keyring).
set -u

KELLU=${1:-}
if [ -z "$KELLU" ] || [ ! -x "$KELLU" ]; then
    echo "usage: $0 /path/to/kellu" >&2
    exit 2
fi
KELLU=$(readlink -f "$KELLU")

if [ -z "${KELLU_REAL_KEYRING_INNER:-}" ]; then
    exec env KELLU_REAL_KEYRING_INNER=1 dbus-run-session -- "$0" "$KELLU"
fi

# ---- inside the throw-away session ----
export XDG_RUNTIME_DIR; XDG_RUNTIME_DIR=$(mktemp -d); chmod 700 "$XDG_RUNTIME_DIR"
export HOME; HOME=$(mktemp -d)
unset XDG_CONFIG_HOME XDG_STATE_HOME XDG_CACHE_HOME
eval "$(echo -n 'throwaway-pw-1' | gnome-keyring-daemon --unlock --components=secrets 2>/dev/null)"
sleep 1

failed=0
check() { # check "description" condition-exit-code
    if [ "$2" -eq 0 ]; then echo "[PASS] $1"; else echo "[FAIL] $1"; failed=1; fi
}
contains() { grep -qF -- "$2" <<<"$1"; }

PROV='sk-or-REALTEST-0123456789abcdef'
INFRA='ghp_REALTEST0123456789abcdefghij'
LEGACY='ghp_LEGACYVALUE0123456789abcdef'
OTHER='firefox-password-do-not-touch'

echo "== set (secret on stdin)"
out=$(printf '%s\n' "$PROV" | "$KELLU" keys set openrouter 2>&1); rc=$?
check "set provider key exits 0" $rc
contains "$out" "stored ai_openrouter_api_key (provider)"; check "set reports the entry and tier" $?
! contains "$out" "$PROV"; check "set never prints the secret" $?
out=$(printf '%s\n' "$INFRA" | "$KELLU" keys set github_token 2>&1); check "set infra key exits 0" $?

echo "== entries from other generations and other applications"
printf '%s' "$LEGACY" | secret-tool store --label 'KAI: github_token' kai github_token
printf '%s' "$LEGACY" | secret-tool store --label 'KAI: ai_old_api_key' kai ai_old_api_key
printf '%s' "$OTHER" | secret-tool store --label 'Firefox' application firefox

echo "== set replaces an existing key"
printf '%s\n' "${PROV}-v2" | "$KELLU" keys set openrouter >/dev/null 2>&1; check "second set exits 0" $?
[ "$(secret-tool lookup kellu ai_openrouter_api_key)" = "${PROV}-v2" ]; check "the new value replaced the old one" $?
printf '%s\n' "$PROV" | "$KELLU" keys set openrouter >/dev/null 2>&1

echo "== list"
out=$("$KELLU" keys list 2>&1); rc=$?
check "list exits 0" $rc
contains "$out" "provider  ai_openrouter_api_key"; check "list shows the provider key (tier provider)" $?
contains "$out" "infra     github_token"; check "list shows the infra key (tier infra)" $?
[ "$(grep -c 'github_token' <<<"$out")" -eq 2 ]; check "list shows the legacy twin too" $?
[ "$(grep -c 'ai_openrouter_api_key' <<<"$out")" -eq 1 ]; check "a replaced key is listed once" $?
contains "$out" "ai_old_api_key"; check "list shows a legacy entry with no twin" $?
! contains "$out" firefox; check "list ignores other applications' items" $?
for secret in "$PROV" "$INFRA" "$LEGACY" "$OTHER"; do
    ! contains "$out" "$secret"; check "list never prints a secret value" $?
done

echo "== cleanup-legacy"
out=$("$KELLU" keys cleanup-legacy 2>&1)
contains "$out" "github_token: has a kellu twin; would be removed with --apply"; check "dry run reports the twin" $?
contains "$out" "ai_old_api_key: kept"; check "dry run keeps the entry with no twin" $?
[ -n "$(secret-tool lookup kai github_token)" ]; check "dry run deleted nothing" $?
out=$("$KELLU" keys cleanup-legacy --apply 2>&1)
contains "$out" "github_token: removed"; check "apply removes the legacy twin" $?
[ -z "$(secret-tool lookup kai github_token)" ]; check "the legacy entry is really gone" $?
[ "$(secret-tool lookup kellu github_token)" = "$INFRA" ]; check "the kellu entry with the same name survived" $?
[ -n "$(secret-tool lookup kai ai_old_api_key)" ]; check "the legacy entry with no twin survived" $?
[ "$(secret-tool lookup application firefox)" = "$OTHER" ]; check "another application's item survived" $?

echo "== rm"
out=$("$KELLU" keys rm openrouter 2>&1); check "rm exits 0" $?
contains "$out" "removed ai_openrouter_api_key"; check "rm reports the entry" $?
out=$("$KELLU" keys list 2>&1)
! contains "$out" "ai_openrouter_api_key"; check "the removed key is no longer listed" $?
[ -z "$(secret-tool lookup kellu ai_openrouter_api_key)" ]; check "the removed key is really gone" $?
"$KELLU" keys rm openrouter >/dev/null 2>&1; check "removing a missing key is not an error" $?

echo "== the child environment is scrubbed (the keyring still works with only what it needs)"
out=$(env -i HOME="$HOME" XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" DBUS_SESSION_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" \
    PATH="$PATH" "$KELLU" keys list 2>&1); check "list works with a minimal environment" $?

if [ $failed -eq 0 ]; then echo "ALL CHECKS PASSED"; else echo "SOME CHECKS FAILED"; fi
exit $failed
