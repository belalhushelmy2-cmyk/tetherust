#!/usr/bin/env bash
# All privileged-looking operations occur only inside an unprivileged user+network+mount namespace.
# The system bus, /run, NM configuration/state and hostname are isolated. No ADB or phone access.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ ${1:-} != --inside ]]; then
    work=$(mktemp -d /tmp/tetherust-nm-XXXXXX)
    trap 'rm -rf "$work"' EXIT
    parent=$(stat -Lc '%i' /proc/self/ns/net)
    export TETHERUST_ADB_DISABLED=1 TETHERUST_PARENT_NETNS="$parent"
    exec_script=$(realpath tests/network_namespace.sh)
    unshare --user --map-root-user --net --mount --pid --fork --uts --ipc "$exec_script" --inside "$work"
    exit
fi
work=$2
mount --make-rprivate /
mount -t tmpfs tmpfs /run
mount -t tmpfs tmpfs /var/lib/NetworkManager
mount -t tmpfs tmpfs /etc/NetworkManager
mount -t sysfs sysfs /sys
mkdir -p /run/NetworkManager /etc/NetworkManager/system-connections
if [[ -e /dev/rfkill ]]; then mount --bind /dev/null /dev/rfkill; fi
ip link set lo up
cat > "$work/bus.conf" <<BUS
<busconfig><type>system</type><listen>unix:path=$work/bus</listen><auth>EXTERNAL</auth><policy context="default"><allow user="*"/><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/></policy></busconfig>
BUS
cat > "$work/nm.conf" <<'NM'
[main]
plugins=keyfile
dns=none
rc-manager=unmanaged
hostname-mode=none
[connectivity]
enabled=false
NM
unset NOTIFY_SOCKET
export DBUS_SYSTEM_BUS_ADDRESS="unix:path=$work/bus"
unset DBUS_SESSION_BUS_ADDRESS
dbus-daemon --nofork --config-file="$work/bus.conf" > "$work/bus.log" 2>&1 & bus_pid=$!
for iteration in $(seq 1 50); do if [[ -S "$work/bus" ]]; then break; fi; sleep .1; done
NetworkManager --no-daemon --config="$work/nm.conf" --pid-file="$work/nm.pid" --log-level=WARN > "$work/nm.log" 2>&1 & nm_pid=$!
trap 'kill "$nm_pid" "$bus_pid" 2>/dev/null || true' EXIT
for iteration in $(seq 1 100); do
    if gdbus call --system --dest org.freedesktop.NetworkManager --object-path /org/freedesktop/NetworkManager --method org.freedesktop.DBus.Properties.Get org.freedesktop.NetworkManager Version >/dev/null 2>&1; then break; fi
    sleep .1
done
runner=(setpriv --bounding-set=-all --inh-caps=-all --ambient-caps=-all --no-new-privs --securebits +noroot,+noroot_locked)
if [[ ${TETHERUST_KERNEL_TUN:-} == 1 ]]; then
    if ! timeout 90s "${runner[@]}" python3 tests/companion_host.py; then cat "$work/nm.log"; exit 1; fi
else
    if ! timeout 45s "${runner[@]}" cargo test --test tun_namespace -- --ignored; then cat "$work/nm.log"; exit 1; fi
fi
