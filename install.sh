#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# apctui installer/upgrader — sets up multi-instance apcupsd on Debian/Ubuntu.
# Run from the project root:  sudo ./install.sh
#
# What it does:
#   1. installs apcupsd (and offers to install cargo if missing)
#   2. BUILDS apctui (cargo build --release, as the invoking user) and
#      installs the binary + polkit policy
#   3. disables the stock single-UPS apcupsd.service (conflicts with instances)
#   4. installs an instanced apcupsd@.service template
#   5. UPGRADE PATH: if instance configs already exist, offers to keep them
#      and stop here — re-running this script is the upgrade procedure
#   6. otherwise: detects attached APC USB UPS units (vendor 051d), pins each
#      by USB serial via udev -> stable /dev/apcups/<name> symlinks
#      (hiddevN numbering shuffles across reboots; serials don't)
#   7. generates one /etc/apcupsd/<name>.conf per unit (NIS ports 3551, 3552, ...)
#   8. asks which unit powers THIS host; all other units get a no-op
#      doshutdown (exit 99 suppresses apccontrol's default system halt)
#   9. enables and starts the instances
#
# Review every generated file before trusting it with your power policy.

set -euo pipefail

UNIT_FILE="/etc/systemd/system/apcupsd@.service"
UDEV_RULES="/etc/udev/rules.d/99-apcupsd-serials.rules"
POLKIT_POLICY="/usr/share/polkit-1/actions/org.apctui.manage.policy"
BIN_DEST="/usr/local/bin/apctui"
APC_VENDOR="051d"
BASE_PORT=3551

die() { echo "error: $*" >&2; exit 1; }
info() { echo "==> $*"; }

[[ $EUID -eq 0 ]] || die "run as root (sudo $0)"
command -v systemctl >/dev/null || die "systemd required"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ASSET_DIR="$SCRIPT_DIR/install"

# Run a command as a given user. For non-root users use a login shell so
# their PATH (incl. ~/.cargo/bin from rustup) applies.
run_as() {
    local u="$1"; shift
    if [[ "$u" == "root" ]]; then
        bash -c "$1"
    else
        sudo -u "$u" -H bash -lc "$1"
    fi
}

# ---------------------------------------------------------------- packages
info "installing apcupsd"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq apcupsd usbutils udev >/dev/null

info "disabling stock single-instance apcupsd.service"
systemctl disable --now apcupsd.service 2>/dev/null || true

# Older Debian/Ubuntu packages gate startup behind /etc/default/apcupsd
if [[ -f /etc/default/apcupsd ]]; then
    sed -i 's/^ISCONFIGURED=.*/ISCONFIGURED=yes/' /etc/default/apcupsd || true
fi

# ---------------------------------------------------------------- build apctui
# Build as the user who invoked sudo (keeps target/ user-owned and uses their
# rustup toolchain if they have one). Falls back to root's cargo, then to an
# apt-installed toolchain.
CANDIDATE_BIN="$SCRIPT_DIR/target/release/apctui"
BUILD_USER="${SUDO_USER:-root}"

ensure_cargo() {
    # Ensure *some* usable cargo exists for $1; offer apt install if not.
    local u="$1"
    if run_as "$u" 'command -v cargo >/dev/null'; then
        return 0
    fi
    if command -v cargo >/dev/null; then
        BUILD_USER="root"
        echo "  note: no cargo for $u; building as root instead (target/ will be root-owned)"
        return 0
    fi
    echo "  cargo not found. apctui needs Rust 1.85+ to build."
    read -rp "  install rustc+cargo from apt now? [Y/n] " ans
    if [[ "${ans:-Y}" =~ ^[Nn] ]]; then
        return 1
    fi
    apt-get install -y -qq cargo rustc >/dev/null || return 1
    BUILD_USER="root"
}

if [[ -f "$SCRIPT_DIR/Cargo.toml" ]]; then
    if ensure_cargo "$BUILD_USER"; then
        ver="$(run_as "$BUILD_USER" 'cargo --version' | awk '{print $2}')"
        if [[ "$(printf '%s\n1.85.0\n' "$ver" | sort -V | head -1)" != "1.85.0" ]]; then
            echo "  WARNING: cargo $ver is older than 1.85; the build may fail."
            echo "  If it does, install a newer toolchain via https://rustup.rs"
        fi
        info "building apctui (cargo build --release, as $BUILD_USER) — first build takes a few minutes"
        if ! run_as "$BUILD_USER" "cd '$SCRIPT_DIR' && cargo build --release"; then
            if [[ -x "$BIN_DEST" ]]; then
                echo "  *** BUILD FAILED. Keeping existing $BIN_DEST ($("$BIN_DEST" --version 2>/dev/null || echo '?'))"
                echo "  *** which may be STALE. Fix the build and re-run."
            else
                die "build failed and no existing binary at $BIN_DEST"
            fi
        fi
    elif [[ -x "$BIN_DEST" ]]; then
        echo "  *** No toolchain; keeping existing $BIN_DEST ($("$BIN_DEST" --version 2>/dev/null || echo '?')) — may be STALE."
    else
        die "no Rust toolchain and no existing binary at $BIN_DEST"
    fi
fi

if [[ -x "$CANDIDATE_BIN" ]]; then
    info "installing apctui -> $BIN_DEST"
    install -m 0755 "$CANDIDATE_BIN" "$BIN_DEST"
    echo "  installed: $("$BIN_DEST" --version 2>/dev/null || echo '?')"
fi

# ---------------------------------------------------------------- polkit policy
# Lets apctui escalate via pkexec with a friendly auth prompt instead of
# requiring you to run the whole TUI as root.
if [[ -f "$ASSET_DIR/org.apctui.manage.policy" ]] && [[ -d /usr/share/polkit-1/actions ]]; then
    info "installing polkit policy -> $POLKIT_POLICY"
    install -m 0644 "$ASSET_DIR/org.apctui.manage.policy" "$POLKIT_POLICY"
else
    echo "  note: skipping polkit policy (no polkit, or policy file missing)."
    echo "        apctui will fall back to sudo for privileged actions."
fi

# ---------------------------------------------------------------- unit file
info "installing $UNIT_FILE"
if [[ -f "$ASSET_DIR/apcupsd@.service" ]]; then
    install -m 0644 "$ASSET_DIR/apcupsd@.service" "$UNIT_FILE"
else
    cat > "$UNIT_FILE" << 'EOF'
[Unit]
Description=APC UPS daemon (%i)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/sbin/apcupsd -b -f /etc/apcupsd/%i.conf -P /run/apcupsd/%i.pid
RuntimeDirectory=apcupsd
RuntimeDirectoryPreserve=yes
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
fi
systemctl daemon-reload

# ---------------------------------------------------------------- upgrade path
# If instance configs already exist, this run is probably an upgrade: the
# binary/unit/policy are refreshed above, and the device setup can be kept.
mapfile -t EXISTING < <(find /etc/apcupsd -maxdepth 1 -name '*.conf' \
    ! -name 'apcupsd.conf' -printf '%f\n' 2>/dev/null | sed 's/\.conf$//' | sort)
if (( ${#EXISTING[@]} > 0 )); then
    echo
    info "existing instance configs found: ${EXISTING[*]}"
    read -rp "Keep current device setup and finish as an upgrade? [Y/n] " ans
    if [[ ! "${ans:-Y}" =~ ^[Nn] ]]; then
        echo
        info "upgrade complete. Verify with:"
        for n in "${EXISTING[@]}"; do
            p="$(awk '/^NISPORT/{print $2; exit}' "/etc/apcupsd/$n.conf" 2>/dev/null)"
            echo "  apcaccess -h 127.0.0.1:${p:-3551}    # $n"
        done
        echo "  apctui"
        exit 0
    fi
    echo "  reconfiguring devices (confs will be regenerated for the names you choose)"
fi

# ---------------------------------------------------------------- detection
info "scanning for APC USB UPS units (vendor $APC_VENDOR)"
declare -a DEVS=() SERIALS=() NAMES=()
shopt -s nullglob
for dev in /dev/usb/hiddev*; do
    walk="$(udevadm info --attribute-walk --name="$dev" 2>/dev/null || true)"
    vendor="$(awk -F'"' '/ATTRS\{idVendor\}/{print $2; exit}' <<< "$walk")"
    [[ "$vendor" == "$APC_VENDOR" ]] || continue
    serial="$(awk -F'"' '/ATTRS\{serial\}/{print $2; exit}' <<< "$walk")"
    [[ -n "$serial" ]] || { echo "  $dev: APC device with no serial — skipping (cannot pin)"; continue; }
    DEVS+=("$dev"); SERIALS+=("$serial")
    echo "  found: $dev  serial=$serial"
done
shopt -u nullglob

[[ ${#DEVS[@]} -gt 0 ]] || die "no APC USB UPS found. Plug units in, or check 'lsusb -d ${APC_VENDOR}:'"

# ---------------------------------------------------------------- naming
echo
echo "Name each unit (lowercase, digits, dashes; e.g. rack-main, rack-aux):"
for i in "${!DEVS[@]}"; do
    while :; do
        read -rp "  name for serial ${SERIALS[$i]}: " name
        [[ "$name" =~ ^[a-z0-9][a-z0-9-]*$ ]] || { echo "  invalid name"; continue; }
        [[ ! " ${NAMES[*]-} " == *" $name "* ]] || { echo "  duplicate name"; continue; }
        NAMES+=("$name"); break
    done
done

echo
echo "Which unit powers THIS host? Its low-battery event will halt the system;"
echo "the others will only log/notify (no-op doshutdown)."
select HOST_UNIT in "${NAMES[@]}" "none (no unit powers this host)"; do
    [[ -n "${HOST_UNIT:-}" ]] && break
done

# ---------------------------------------------------------------- udev rules
info "writing $UDEV_RULES"
{
    echo "# generated by apctui install.sh — pins APC UPS units by USB serial"
    for i in "${!NAMES[@]}"; do
        printf 'KERNEL=="hiddev*", SUBSYSTEMS=="usb", ATTRS{idVendor}=="%s", ATTRS{serial}=="%s", SYMLINK+="apcups/%s"\n' \
            "$APC_VENDOR" "${SERIALS[$i]}" "${NAMES[$i]}"
    done
} > "$UDEV_RULES"
udevadm control --reload
udevadm trigger --subsystem-match=usbmisc
sleep 1
for n in "${NAMES[@]}"; do
    [[ -e "/dev/apcups/$n" ]] && echo "  /dev/apcups/$n -> $(readlink -f "/dev/apcups/$n")" \
        || echo "  warning: /dev/apcups/$n did not appear; check 'udevadm test'"
done

# ---------------------------------------------------------------- configs
port=$BASE_PORT
for i in "${!NAMES[@]}"; do
    n="${NAMES[$i]}"
    conf="/etc/apcupsd/$n.conf"
    info "writing $conf (NIS port $port)"

    scriptdir="/etc/apcupsd"
    if [[ "$n" != "$HOST_UNIT" ]]; then
        # Non-host units must never halt this machine. apccontrol runs an
        # event script from SCRIPTDIR if present; exit code 99 suppresses
        # the built-in action (system shutdown).
        scriptdir="/etc/apcupsd/$n.d"
        mkdir -p "$scriptdir"
        cp -n /etc/apcupsd/apccontrol "$scriptdir/apccontrol" 2>/dev/null || true
        cat > "$scriptdir/doshutdown" << 'EOF'
#!/bin/sh
# Installed by apctui: this UPS does not power this host.
# Exit 99 tells apccontrol to skip the default shutdown action.
logger -t apcupsd "doshutdown suppressed: UPS does not power this host"
exit 99
EOF
        chmod +x "$scriptdir/doshutdown"
    fi

    cat > "$conf" << EOF
## generated by apctui install.sh — $(date -Is)
## unit: $n   serial: ${SERIALS[$i]}
UPSNAME $n
UPSCABLE usb
UPSTYPE usb
DEVICE /dev/apcups/$n

# power-off policy (host unit only acts on these; tune to taste)
ONBATTERYDELAY 6
BATTERYLEVEL 10
MINUTES 5
TIMEOUT 0

# network information server (apctui and net clients connect here)
NETSERVER on
NISIP 0.0.0.0
NISPORT $port

# per-instance state so units don't trample each other
SCRIPTDIR $scriptdir
PWRFAILDIR /etc/apcupsd
NOLOGINDIR /etc
EVENTSFILE /var/log/apcupsd-$n.events
EVENTSFILEMAX 25
STATTIME 60
STATFILE /var/log/apcupsd-$n.status
LOCKFILE /var/lock

KILLDELAY 0
EOF
    port=$((port + 1))
done

# ---------------------------------------------------------------- enable
echo
for n in "${NAMES[@]}"; do
    info "enabling apcupsd@$n"
    systemctl enable --now "apcupsd@$n"
done

echo
info "done. Verify with:"
port=$BASE_PORT
for n in "${NAMES[@]}"; do
    echo "  apcaccess -h 127.0.0.1:$port    # $n"
    port=$((port + 1))
done
echo "  apctui   # or: apctui --ups ${NAMES[0]}=127.0.0.1:$BASE_PORT ..."
echo
echo "NOTE: NIS has no authentication. NISIP is 0.0.0.0 so LAN clients can"
echo "connect; restrict with your firewall, e.g.:"
echo "  ufw allow from 192.168.1.0/24 to any port $BASE_PORT:$((BASE_PORT + ${#NAMES[@]} - 1)) proto tcp"
