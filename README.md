# apctui

apctui is a terminal dashboard for [apcupsd](http://www.apcupsd.org/). It pulls the status of multiple APC UPS units into one screen and handles the awkward multi‑instance configuration that apcupsd requires. Built in Rust against [ratatui](https://ratatui.rs/) and released under GPL‑3.0‑or‑later.

---

## Why this exists

`apcupsd` runs a *single* daemon by default. Plug in a second UPS and suddenly only whichever USB port enumerated first is monitored; reboot the machine and the winner might change. A proper multi‑UPS setup needs one service per unit, separate configs, persistent device names via udev, and clear boundaries about which UPS can cut power to the host. Doing that by hand is error‑prone. apctui’s installer writes the udev rules, generates per‑instance configs on ascending NIS ports, asks which unit actually powers the machine, and enables the services. The dashboard then collects every instance so you can see what’s happening without juggling multiple consoles.

### Why not NUT?

If you’re wondering why this isn’t built on [Network UPS Tools](https://networkupstools.org/), the answer is that this project started as an experiment. There weren’t any comparable consoles built on apcupsd, and testing Anthropic’s Fable model required something novel. NUT covers more brands and is a better long‑term answer for many environments. apctui exists because experimenting with AI‑assisted development on a niche stack was interesting.

## What’s on screen

Cards arrange themselves to fill the screen: one unit gets the full area, two
sit side by side, three or four form a 2×2 grid (a three-unit grid leaves a
"no device" placeholder in the fourth cell). Five or more units, or a terminal
too small for the grid, drop to a compact one-line-per-unit table.

Every card includes:

- live status (`ONLINE`, `ONBATT`, `CAL`…), estimated runtime, output watts and load as gauges
- a history plot sampled every two seconds (load is bars, battery charge is a line)
- color shifts on the load gauge: green → yellow → red as you approach capacity
- a red `COMMLOST` badge if apctui can’t reach a daemon; the dashboard keeps running when one unit drops

Press `↵` on a unit to view detailed metrics such as input and output voltage, line frequency, battery date and firmware revision.

## Installation

Clone the repo and run the installer. You’ll need a Rust toolchain (1.85+ recommended). The script builds the binary as your user, detects all connected APC USB devices, and walks you through naming them and picking the one that powers the host. It writes udev rules keyed off the devices’ serial numbers, generates configs, and enables the systemd services. Re‑running the installer upgrades and refreshes everything.

```bash
git clone https://github.com/b3p3k0/apctui.git
cd apctui
sudo ./install.sh
```

On the first run it offers to install `cargo` via `apt` if you don’t already have Rust. The installer writes per‑device configs in `/etc/apcupsd/` on NIS ports starting at 3551. Units that don’t power the host get a no‑op `doshutdown` script so a low battery in your network rack doesn’t halt your server. Upgrading is as simple as pulling the repo and running the installer again.

## Running the TUI

Invoking `apctui` scans `/etc/apcupsd/*.conf`, connects to each NIS endpoint, and shows you the dashboard. At startup it prints where it found the unit list, so misconfigured setups are obvious. There are a few useful flags:

```bash
apctui            # normal dashboard with color
apctui --basic    # force pure ASCII (no color blocks)
apctui --probe    # one‑shot status dump of all units, no TUI
```

To monitor remote hosts, or hide units you don’t care about, create `~/.config/apctui/config.toml` (see `examples/apctui.toml` in the repo) and define additional `[ups]` entries or mark existing ones as `hidden = true`.

### Keyboard shortcuts

| key         | action                                   |
|-------------|------------------------------------------|
| `j`/`k`     | move selection up/down                   |
| `↵`         | open detail view for the selected UPS    |
| `c`         | open configuration editor                |
| `s`         | service control (start/stop/enable…)     |
| `g`         | generate client config bundle            |
| `o`         | notification options                     |
| `e`         | view event log                           |
| `b`         | toggle ASCII mode on the fly             |
| `p`         | pause/resume polling                     |
| `q`         | quit                                     |

## Configuration editing

Press `c` from the dashboard to edit every apcupsd instance at once. The editor opens one tab per unit; `Tab` switches between them. Fields are typed: enums cycle through valid values, booleans toggle, integers are range‑checked. Validation runs on every change and enforces apcupsd’s real rules — for example `UPSTYPE net` without a `host:port` device is an error and disabling all shutdown triggers gets you a warning. Save with `s` and apctui shows you a unified diff before anything touches disk. Only the directive you changed changes; comments, blank lines, and odd whitespace are preserved byte‑for‑byte. The actual write happens through a privileged helper (`pkexec`; sudo fallback). It re‑validates as root, refuses to write a config with errors, backs up the old file with a timestamp, writes atomically, and restarts the daemon. The TUI itself never runs as root.

## Service control

`s` opens a view where you can start, stop, restart, enable, or disable services per instance. Each action has a confirmation step that spells out what stopping means: monitoring ends and shutdown protection ends. Choose wisely. apctui uses systemd behind the scenes and shows the current state of each service.

## Network clients

Machines powered by your UPSes but plugged into someone else’s USB port can run apcupsd in net‑client mode against this host. Press `g` on a unit to generate a client configuration bundle. apctui writes a directory in `~/apctui-client-bundles` containing the config file and installation steps. The client configs deliberately use more conservative shutdown thresholds than the master so clients finish shutting down before the master cuts power.

## Notifications

Push notifications are optional and off by default. Hit `o` and paste your [Pushbullet](https://www.pushbullet.com/) access token, then pick which events you care about. A test push is available (`t`) to confirm it works. apctui sends a notification when:

- a unit switches to battery power (includes load and estimated runtime)
- line power returns (includes charge level)
- a unit stops answering (after three consecutive failed polls to avoid false positives)
- a lost unit comes back

Repeat events for the same unit are rate‑limited (default 60 seconds, configurable). Notifications run on a background thread so network hiccups don’t freeze the UI. Settings persist to `~/.config/apctui/config.toml` under `[notifications]`. The token is stored in plaintext; apctui sets permissions to 0600 but it’s your home directory — treat it accordingly. Saving notifications rewrites only the `[notifications]` section; your hand‑written `[[ups]]` entries and their comments stay untouched.

## ASCII mode

If you’re on a serial console, using a screen reader, or need to capture logs without unicode blocks, pass `--basic` or press `b`. This forces pure 7‑bit ASCII output. The test suite renders every view and fails if a single non‑ASCII byte appears, so ASCII mode isn’t an afterthought.

## Limitations

- **No authentication on NIS.** The apcupsd net server is read‑only, but anyone who can reach port 3551 can read status. The generated configs bind `0.0.0.0` so LAN clients work; firewall accordingly. The installer prints a ready‑made `ufw` rule.
- **Blank serial numbers break udev pinning.** Some low‑end APC models ship without a USB serial. udev can’t differentiate them, so per‑unit rules won’t stick. Check with `udevadm info -a -n /dev/usb/hiddev0` before trusting a multi‑unit setup.
- **Debian/Ubuntu only.** The installer handles the multi‑instance dance on Debian‑derivatives via systemd. The TUI itself just needs NIS endpoints and will monitor anything; you can manage services manually on other distros.
- A unit with `NETSERVER off` can’t be monitored. It shows in the services view but not on the dashboard.

## License

GPL‑3.0‑or‑later. Full text in [LICENSE](LICENSE).
