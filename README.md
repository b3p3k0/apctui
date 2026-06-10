# apctui

A slick terminal UI to **monitor and manage [apcupsd](http://www.apcupsd.org/)**,
built for hosts running **multiple APC UPS units** (designed and tested for up
to four). It watches every unit live, edits their configs safely, controls
their services, and generates ready-to-deploy configs for networked clients —
all without leaving the terminal.

Rich mode gives btop-style gauges, gradients, and sparklines. `--basic` (or
`NO_COLOR` / `TERM=dumb`) degrades to **pure ASCII** for serial consoles and
log capture. Color depth auto-detects: truecolor → 256 → 16 → mono.

```
 apctui │ dashboard  2 units
╭▌ rack-main Smart-UPS 1500 ⚡ ONLINE ──────────────────────────────────────╮
│load  42% ██████████████▊            batt  93% ████████████████████████▌   │
│ line 121.5V batt 27.3V out 378.0W run 22.0m xfers 3                        │
│       ▁▁▂▃▃▄▄▅▅▆▆▇███████ ▁▁▂▃▃▄▄▅▅▆▆▇███████ ▁▁▂▃▃▄▄▅▅                    │
│ load ████████████████████████████████████████████████████                 │
╰───────────────────────────────────────────────────────────────────────────╯
 ↵ detail  c config  s services  g client-gen  e events  b basic  p pause  ? help  q quit
```

## Features

- **Live dashboard** — one card per UPS: status, load/battery gauges, output
  watts, runtime, transfer count, and a btop-style rolling history chart —
  **load as bars, battery charge as a line** — sampled every 2 s by default
  (`--interval`). Auto-collapses to a compact one-line-per-UPS table when the
  terminal is short.
- **Detail view** — the full set of NIS fields for the selected unit.
- **Config editor** — a centralized, structured editor for every instance's
  `apcupsd.conf`, **one tab per unit** (Tab/Shift-Tab or ←/→ to switch):
  - typed fields (enums cycle, booleans toggle, integers range-check)
  - **live validation** with apcupsd-specific rules (e.g. `UPSTYPE net`
    requires `DEVICE host:port`; warns when no shutdown trigger is set)
  - a **diff preview before saving**
  - a **round-trip parser** that preserves every comment, blank line, and bit
    of whitespace — only the directives you change are touched
  - saves by escalating per-action (pkexec, falling back to sudo), then
    restarts the affected daemon (apcupsd needs a restart; it ignores SIGHUP)
- **Service control** — discovers configured instances, shows active/enabled
  state and NIS endpoint, and starts/stops/restarts/enables/disables them with
  a confirmation step for destructive actions.
- **Client config generator** — one tab per unit; produces an `apcupsd.conf`
  for a machine that draws power from that UPS but runs apcupsd as a network
  client, with a live preview and a deploy bundle (config + install
  instructions). Client
  shutdown thresholds are auto-set more conservative than the master's, so
  clients power down before the master cuts UPS output.
- **Events viewer** — tails the apcupsd event logs.

## Install / upgrade (Debian/Ubuntu)

One command from the project root — it builds apctui, installs the binary,
and sets up multi-instance apcupsd:

```sh
sudo ./install.sh
```

**Upgrading:** pull/extract the new source and re-run `sudo ./install.sh`.
It rebuilds, reinstalls the binary, refreshes the systemd unit and polkit
policy, then offers to keep your existing device setup untouched.

The build runs as *your* user (not root), using your toolchain — rustup or
distro cargo, Rust 1.85+. If no toolchain is found the script offers to
install one from apt.

Manual build, if you prefer:

```sh
cargo build --release
sudo install -m755 target/release/apctui /usr/local/bin/
```

## Run

```sh
apctui                                   # auto-discovers /etc/apcupsd/*.conf
apctui --ups rack-main=127.0.0.1:3551 --ups rack-aux=127.0.0.1:3552
apctui --basic                           # plain-monitor mode (pure ASCII)
apctui --probe                           # one-shot status dump, no TUI
```

With no flags and no config file, apctui **discovers every instance** in
`/etc/apcupsd/*.conf` and connects to each one's NIS endpoint — so after
`install.sh` you just run `apctui` and all your units appear. The CGI-tool
configs the apcupsd package ships (`hosts.conf`, `multimon.conf`) are
excluded, and you can hide any other instance with an ignore list in
`~/.config/apctui/config.toml`:

```toml
[discovery]
ignore = ["closet-test"]
```

The resolution
order is: `--ups` flags → `--config FILE` → `~/.config/apctui/config.toml` →
`/etc/apcupsd/*.conf` discovery → a single local fallback on `127.0.0.1:3551`.

You can still define units explicitly in `~/.config/apctui/config.toml` (see
`examples/apctui.toml`) — useful for monitoring **remote** UPS hosts that have
no config file on this machine:

```toml
[[ups]]
name = "rack-main"
addr = "127.0.0.1:3551"

[[ups]]
name = "rack-aux"
addr = "127.0.0.1:3552"
```

### Keys

| Context   | Keys |
|-----------|------|
| Dashboard | `↵`/`l` detail · `j`/`k` select · `c` config · `s` services · `g` client-gen · `e` events · `b` basic · `p` pause · `?` help · `q` quit |
| Editor    | `⇥` switch unit · `↑↓` field · `↵` edit · `space` toggle/cycle · `d` diff · `s` save · `esc` close |
| Services  | `↑↓` select · `r` restart · `S` start · `x` stop · `e` enable · `d` disable · `R` rescan · `esc` back |
| Client-gen| `⇥` switch unit · `↑↓` field · `↵` edit · `w` write bundle · `esc` back |

## Multi-UPS host setup

apcupsd runs **one daemon per UPS**. `./install.sh` automates the whole
multi-instance setup on Debian/Ubuntu:

- builds and installs the `apctui` binary, installs apcupsd, disables the
  stock single-instance service
- installs an instanced `apcupsd@.service`
- installs a **polkit policy** so the editor can escalate with a friendly auth
  prompt instead of running the whole TUI as root
- **pins each UPS by USB serial number** via udev → `/dev/apcups/<name>`
  (raw `hiddevN` numbering shuffles across reboots; with identical units a
  blank-`DEVICE` autodetect would grab an arbitrary one)
- generates `/etc/apcupsd/<name>.conf` per unit on NIS ports 3551, 3552, …
- asks **which unit powers this host** — only that unit may halt the system;
  the others get a no-op `doshutdown` (exit 99 suppresses apccontrol's default)

```sh
sudo ./install.sh
```

Review the generated files before trusting them with your power policy. Two
things worth checking on real hardware:

- `udevadm info -a -n /dev/usb/hiddev0` — confirm your units expose vendor
  `051d` and a populated `serial` attribute (some cheap models ship blank
  serials, which breaks serial pinning).
- that each UPS reports a **distinct** serial.

## Security notes

- apcupsd's NIS protocol is **unauthenticated, read-only status**. The
  generated configs bind `0.0.0.0` so LAN clients work; scope access with your
  firewall (the installer prints a ready-made `ufw` rule).
- apctui runs **unprivileged**. Config writes and service actions are performed
  by a separate `apctui apply` / `apctui service` helper invoked via pkexec
  (or sudo). That helper **re-validates** the config as root and refuses to
  write anything with errors, makes a **timestamped backup**, and writes
  **atomically** (temp file + rename).
- apcupsd does **not** reload on SIGHUP — applying changes restarts the daemon,
  and the UI tells you so.

## Development without hardware

```sh
cargo run --example mock_nis -- 3551 rack-main &
cargo run --example mock_nis -- 3552 rack-aux onbatt &   # cycles ONLINE/ONBATT/LOWBATT
cargo run -- --ups rack-main=127.0.0.1:3551 --ups rack-aux=127.0.0.1:3552
```

## Tests

```sh
cargo test
```

Covers the NIS protocol round-trip, the config parser's byte-exact
round-tripping, schema validation rules, the diff engine, service discovery,
client-config generation, and **rendering of every view** (via ratatui's
`TestBackend`), including a guarantee that **basic mode emits only ASCII**.

## License

GPL-3.0-or-later. See `LICENSE`.
