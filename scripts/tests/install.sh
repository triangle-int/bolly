#!/usr/bin/env bash
# Sandboxed installer behavior tests: no network, services, or user files.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
spawned_pids=()
cleanup() {
  for pid in "${spawned_pids[@]}"; do kill -9 "$pid" 2>/dev/null || true; done
  rm -rf "$tmp"
}
trap cleanup EXIT
mock_bin="$tmp/mock-bin"
mkdir -p "$mock_bin"

cat > "$mock_bin/uname" <<'MOCK'
#!/usr/bin/env bash
case "$1" in
  -s) printf '%s\n' "${MOCK_OS:?}" ;;
  -m) printf '%s\n' "${MOCK_ARCH:?}" ;;
  *) exit 2 ;;
esac
MOCK

cat > "$mock_bin/curl" <<'MOCK'
#!/usr/bin/env bash
set -eu
printf 'curl %s\n' "$*" >> "$MOCK_CALLS"
case "$*" in
  *' -o '*)
    out=''
    while [[ $# -gt 0 ]]; do
      if [[ "$1" = -o ]]; then out=$2; break; fi
      shift
    done
    [[ -n "$out" ]]
    cat > "$out" <<'BIN'
#!/usr/bin/env bash
printf 'direct binary start\n' >> "${MOCK_CALLS:?}"
BIN
    ;;
  *'-fsSIL '*) printf 'location: https://github.com/triangle-int/nolune/releases/download/v0.33.0/artifact\r\n' ;;
  *'/healthz'*) [[ "${MOCK_HEALTHY:-1}" = 1 ]] ;;
  *'/api/health'*) exit 42 ;;
  *) ;;
esac
MOCK

cat > "$mock_bin/launchctl" <<'MOCK'
#!/usr/bin/env bash
printf 'launchctl %s\n' "$*" >> "$MOCK_CALLS"
exit 0
MOCK

cat > "$mock_bin/systemctl" <<'MOCK'
#!/usr/bin/env bash
printf 'systemctl %s\n' "$*" >> "$MOCK_CALLS"
exit 0
MOCK

cat > "$mock_bin/id" <<'MOCK'
#!/usr/bin/env bash
if [[ "${1:-}" = -u ]]; then printf '%s\n' "${MOCK_UID:-501}"; else /usr/bin/id "$@"; fi
MOCK

cat > "$mock_bin/pgrep" <<'MOCK'
#!/usr/bin/env bash
printf 'pgrep %s\n' "$*" >> "$MOCK_CALLS"
if [[ -n "${MOCK_OLD_PID:-}" ]]; then
  # A decoy first exposes unsafe "first pgrep match" implementations.
  printf '999998\n%s\n' "$MOCK_OLD_PID"
else
  exit 1
fi
MOCK

cat > "$mock_bin/ps" <<'MOCK'
#!/usr/bin/env bash
printf 'ps %s\n' "$*" >> "$MOCK_CALLS"
printf '999997 /usr/bin/helper %s\n' "${NOLUNE_DIR:?}/bin/nolune"
printf '999996 %s --helper\n' "${NOLUNE_DIR:?}/bin/nolune"
if [[ -n "${MOCK_OLD_PID:-}" ]] && kill -0 "$MOCK_OLD_PID" 2>/dev/null; then
  state=$(/bin/ps -p "$MOCK_OLD_PID" -o stat= 2>/dev/null || true)
  if [[ "$state" != *Z* ]]; then
    printf '%s %s\n' "$MOCK_OLD_PID" "${NOLUNE_DIR:?}/bin/nolune"
  fi
fi
MOCK

for cmd in kill open xdg-open; do
  cat > "$mock_bin/$cmd" <<'MOCK'
#!/usr/bin/env bash
printf '%s %s\n' "$(basename "$0")" "$*" >> "$MOCK_CALLS"
[[ "$(basename "$0")" != pgrep ]]
MOCK
done

cat > "$mock_bin/sleep" <<'MOCK'
#!/usr/bin/env bash
exit 0
MOCK
chmod +x "$mock_bin"/*
mock_bin_no_systemd="$tmp/mock-bin-no-systemd"
mkdir -p "$mock_bin_no_systemd"
for mock in "$mock_bin"/*; do
  [[ $(basename "$mock") = systemctl ]] || ln -s "$mock" "$mock_bin_no_systemd/$(basename "$mock")"
done
for cmd in bash basename cat chmod grep head mkdir mv rm sed seq tail tr whoami; do
  [[ -e "$mock_bin_no_systemd/$cmd" ]] || ln -s "$(command -v "$cmd")" "$mock_bin_no_systemd/$cmd"
done

assert_contains() {
  local file=$1 expected=$2
  if ! grep -Fq -- "$expected" "$file"; then
    printf 'FAIL: missing call: %s\n--- calls ---\n' "$expected" >&2
    cat "$file" >&2
    exit 1
  fi
}

wait_for_call() {
  local file=$1 expected=$2
  for _ in $(seq 1 100); do
    grep -Fq -- "$expected" "$file" && return 0
    /bin/sleep 0.02
  done
  printf 'FAIL: timed out waiting for call: %s\n--- calls ---\n' "$expected" >&2
  cat "$file" >&2
  exit 1
}

assert_absent() {
  local file=$1 unexpected=$2
  if grep -Fq -- "$unexpected" "$file"; then
    printf 'FAIL: unexpected call: %s\n--- calls ---\n' "$unexpected" >&2
    cat "$file" >&2
    exit 1
  fi
}

assert_status() {
  local name=$1 expected=$2 actual
  actual=$(cat "$tmp/$name/status")
  if [[ "$actual" -ne "$expected" ]]; then
    printf 'FAIL: %s exited %s, expected %s\n--- output ---\n' "$name" "$actual" "$expected" >&2
    cat "$tmp/$name/output" >&2
    exit 1
  fi
}

run_installer() {
  local name=$1 os=$2 arch=$3 healthy=$4 old_pid=${5:-} uid=${6:-501} with_systemd=${7:-1}
  local home="$tmp/$name/home" data="$tmp/$name/data" calls="$tmp/$name/calls"
  if [[ "$old_pid" = spawn ]]; then
    old_pid=$(python3 -c 'import subprocess; print(subprocess.Popen(["/bin/sleep", "300"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True).pid)')
    spawned_pids+=("$old_pid")
    printf '%s\n' "$old_pid" > "$tmp/$name-old-pid"
  elif [[ "$old_pid" = stubborn ]]; then
    old_pid=$(python3 -c 'import subprocess; print(subprocess.Popen(["python3", "-c", "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(300)"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True).pid)')
    spawned_pids+=("$old_pid")
    printf '%s\n' "$old_pid" > "$tmp/$name-old-pid"
    /bin/sleep 0.1
  fi
  local selected_path="$mock_bin:/usr/bin:/bin:/usr/sbin:/sbin"
  if [[ "$with_systemd" != 1 ]]; then
    selected_path=$mock_bin_no_systemd
  fi
  mkdir -p "$home"
  : > "$calls"
  set +e
  HOME="$home" NOLUNE_DIR="$data" SHELL=/bin/bash \
    MOCK_OS="$os" MOCK_ARCH="$arch" MOCK_HEALTHY="$healthy" MOCK_OLD_PID="$old_pid" MOCK_UID="$uid" MOCK_CALLS="$calls" \
    NOLUNE_SYSTEMD_SYSTEM_DIR="$tmp/$name/systemd-system" \
    PATH="$selected_path" \
    bash "$root/scripts/install.sh" > "$tmp/$name/output" 2>&1
  status=$?
  set -e
  printf '%s\n' "$status" > "$tmp/$name/status"
}

# macOS: native service manager owns startup; health uses the public endpoint.
run_installer macos Darwin arm64 1 spawn
assert_status macos 0
assert_contains "$tmp/macos/calls" 'launchctl bootstrap gui/501 '
assert_contains "$tmp/macos/calls" 'launchctl kickstart -k gui/501/dev.nolune.nolune'
assert_contains "$tmp/macos/calls" 'launchctl print gui/501/dev.nolune.nolune'
old_pid=$(cat "$tmp/macos-old-pid")
old_state=$(/bin/ps -p "$old_pid" -o stat= 2>/dev/null || true)
if kill -0 "$old_pid" 2>/dev/null && [[ "$old_state" != *Z* ]]; then
  echo 'FAIL: installer left the exact old Nolune process running' >&2
  exit 1
fi
if ! grep -Fq "stopping unmanaged nolune process (PID: $old_pid)" "$tmp/macos/output"; then
  echo 'FAIL: installer did not stop the unmanaged prior process' >&2
  exit 1
fi
assert_contains "$tmp/macos/calls" 'ps -axo pid=,command='
assert_contains "$tmp/macos/calls" '/healthz'
assert_absent "$tmp/macos/calls" '/api/health'
assert_absent "$tmp/macos/calls" 'direct binary start'
if grep -Eq 'auth token:|/auth\?token=' "$tmp/macos/output"; then
  echo 'FAIL: installer printed the auth token' >&2
  exit 1
fi

# Fresh installs and their generated updater use the Nolune distribution contract.
assert_contains "$tmp/macos/calls" 'https://github.com/triangle-int/nolune/releases/latest/download/nolune-server-aarch64-apple-darwin'
assert_contains "$tmp/macos/home/Library/LaunchAgents/dev.nolune.nolune.plist" '<key>NOLUNE_HOME</key>'
assert_contains "$tmp/macos/data/bin/update" 'nolune-server-'
bash -n "$tmp/macos/data/bin/update"
MOCK_CALLS="$tmp/macos/calls" PATH="$mock_bin:$PATH" bash "$tmp/macos/data/bin/update" > "$tmp/macos/update-output"
assert_contains "$tmp/macos/update-output" 'already at v0.33.0'

# Re-running the macOS installer against the same home/data remains idempotent.
run_installer macos Darwin arm64 1
assert_status macos 0
assert_contains "$tmp/macos/calls" 'launchctl bootstrap gui/501 '
assert_contains "$tmp/macos/calls" '/healthz'

# Root Linux uses and verifies the system service without touching real /etc.
run_installer linux-root Linux x86_64 1 '' 0
assert_status linux-root 0
assert_contains "$tmp/linux-root/calls" 'systemctl daemon-reload'
assert_contains "$tmp/linux-root/calls" 'systemctl enable nolune'
assert_contains "$tmp/linux-root/calls" 'systemctl restart nolune'
assert_contains "$tmp/linux-root/calls" 'systemctl is-active --quiet nolune'
[[ -f "$tmp/linux-root/systemd-system/nolune.service" ]]

# Non-root Linux uses and verifies the user systemd service.
run_installer linux Linux x86_64 1
assert_status linux 0
assert_contains "$tmp/linux/calls" 'systemctl --user daemon-reload'
assert_contains "$tmp/linux/calls" 'systemctl --user enable nolune'
assert_contains "$tmp/linux/calls" 'systemctl --user restart nolune'
assert_contains "$tmp/linux/calls" 'systemctl --user is-active --quiet nolune'
assert_contains "$tmp/linux/calls" '/healthz'
assert_absent "$tmp/linux/calls" 'direct binary start'

# Re-running Linux against the same home/data remains idempotent.
run_installer linux Linux x86_64 1
assert_status linux 0
assert_contains "$tmp/linux/calls" 'systemctl --user enable nolune'

# Linux without systemd uses the explicit direct-process fallback.
run_installer linux-fallback Linux x86_64 1 '' 501 0
assert_status linux-fallback 0
wait_for_call "$tmp/linux-fallback/calls" 'direct binary start'

# A process that ignores SIGTERM blocks service startup and fails closed.
run_installer stubborn Darwin arm64 1 stubborn
[[ $(cat "$tmp/stubborn/status") -ne 0 ]]
assert_absent "$tmp/stubborn/calls" 'launchctl bootstrap gui/501 '
if ! grep -Fq 'could not stop the existing Nolune process' "$tmp/stubborn/output"; then
  echo 'FAIL: stubborn prior process did not produce a clear failure' >&2
  exit 1
fi

# A service that never becomes healthy makes installation fail honestly.
run_installer unhealthy Darwin arm64 0
[[ $(cat "$tmp/unhealthy/status") -ne 0 ]]
assert_contains "$tmp/unhealthy/calls" '/healthz'
if grep -Fq 'installation complete' "$tmp/unhealthy/output"; then
  echo 'FAIL: unhealthy installation claimed completion' >&2
  exit 1
fi

printf 'All installer shell tests passed.\n'
