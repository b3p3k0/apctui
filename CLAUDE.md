# apctui — agent handoff

You are taking over as development partner on apctui, a Rust/ratatui TUI that
monitors and manages multiple apcupsd instances. The owner (Kevin, GitHub
`b3p3k0`) directs the work; you implement, test, and prove. Read this whole
file before touching code.

## Working relationship

- Tell it like it is. No sugar-coating, no false praise, no hedging. If
  Kevin's idea has a flaw, say so directly and propose better.
- Deliver complete files or complete diffs — never partial snippets that
  force manual transcription.
- Every command you give him must be copy-pasteable with zero placeholders.
  Real paths, real filenames, real branch names.
- Validate before declaring done: build clean, full test suite green, and
  when behavior is user-visible, demonstrate it (run it against the mocks).
- When something fails, own it, find the root cause, and add a guardrail so
  the class of failure can't recur. That pattern built half this codebase.

## Project facts

- Repo: https://github.com/b3p3k0/apctui — GPL-3.0-or-later
- Working tree: `/home/kevin/DEV/apctui`; `patches/` inside it is gitignored
  scratch space
- Rust, MSRV 1.85 (`edition 2021`), ratatui 0.29
- `apctui --version` reports semver + git hash (e.g. `0.5.0 (gb928358)`)
  via `build.rs`. Bare semver proved useless for "is this binary current" —
  always verify the hash after installs.
- Deployment: `sudo ./install.sh` from repo root builds (as the invoking
  user), installs to `/usr/local/bin/apctui`, refreshes the systemd template
  and polkit policy, and offers to keep the existing device setup (that path
  IS the upgrade procedure).
- Production host: `ktec-minipc`, Ubuntu 26.04, two real APC units on USB,
  instances `apc0` (NIS 3551) and `apc1` (NIS 3552), pinned by USB serial via
  udev to `/dev/apcups/<name>`.

## Architecture map

One daemon per UPS is apcupsd's model; everything follows from that.

- `src/main.rs` — CLI (clap), poller spawn (one thread per unit), event loop:
  drain updates → `app.apply()` → `app.tick()` → draw → key input.
- `src/app.rs` — all state and key handling. `UpsPanel` per unit (status,
  history, notification baselines). Views: Dashboard, Detail, Editor,
  Services, ClientGen, Events, Options, Help. Editor and ClientGen are
  tabbed (one tab per instance/unit).
- `src/poller.rs` — background NIS polling, `Update` over mpsc.
- `src/nis.rs` — apcupsd NIS protocol client (length-prefixed records).
- `src/config/` — apcupsd.conf parser (byte-exact round-trip: comments,
  whitespace, ordering all preserved), typed schema (~28 directives),
  validation with apcupsd's real coherence rules, diff engine.
- `src/privileged.rs` — `apctui apply` helper run via pkexec (sudo
  fallback): re-validates as root, refuses errors, timestamped backup,
  atomic write, then restarts the unit. Restart, not reload — apcupsd
  ignores SIGHUP. The TUI itself never runs as root.
- `src/service.rs` — instance discovery from `/etc/apcupsd/*.conf`
  (excludes `apcupsd`, `hosts`, `multimon`), systemd control.
- `src/registry.rs` — unit resolution: CLI `--ups` flags > config file
  `[[ups]]` > discovery, with `[discovery] ignore` list support.
- `src/clientgen.rs` + `src/netutil.rs` — network-client config generator.
  Master address prefills this host's detected private IP; priority
  100.64/10 > 10/8 > 172.16/12 > 192.168/16 (overlay networks win, per
  owner spec). Loopback master triggers a bold form warning.
- `src/options.rs` — app settings in `~/.config/apctui/config.toml`,
  written with `toml_edit` so user `[[ups]]` entries and comments survive
  byte-for-byte. chmod 600 when a token is present.
- `src/notify.rs` — Pushbullet push notifications. Detection lives in
  `App::apply` (pure, testable): transitions only — ONBATT/online, comm
  lost (3 consecutive bad polls, both flavors: NIS failure AND daemon-
  reported STATUS COMMLOST), comm restored. Delivery on a worker thread
  with per-(unit,event) cooldown. `APCTUI_PUSHBULLET_URL` env overrides
  the endpoint for testing.
- `src/theme.rs`, `src/ui/` — rendering. Truecolor/256/mono detection.

## Invariants you must not break

1. **Basic mode is pure 7-bit ASCII.** `--basic`, `b` key, `NO_COLOR`, or
   `TERM=dumb`. Tests render every view and fail on a single non-ASCII
   byte. All glyphs go through `theme.rs` helpers (`g_dot`, `g_check`,
   `enum_open`, ...) — never hardcode Unicode in a view.
2. **Config round-trip is byte-exact.** Users hand-annotate their configs;
   the parser must preserve everything it didn't change. Same rule for the
   user's own config.toml (`toml_edit`, never plain serialize).
3. **Charts anchor newest-sample-RIGHT** (btop convention), including
   partially-filled history. There's a regression test.
4. **Exactly one notification per event per machine.** The notifier is a
   flock singleton (`~/.config/apctui/notifier.lock`); extra instances run
   standby and take over within ~10s of the sender exiting. Two gotchas,
   both regression-tested: rebuilding on options-save must drop the old
   notifier BEFORE spawning (same-process flock self-deadlocks into
   standby), and the options test push uses `spawn_with_lock(_, None)`.
   Unit tests must never touch the default lock path — a running apctui
   instance holds it and your test becomes environment-dependent.
5. **Silent state is forbidden.** Unsaved changes get a dirty banner and a
   close prompt; armed/standby notifier shows in the header; delivered
   pushes toast; failures toast. If you add state a user can't see, you've
   added a bug they'll trip over.
6. **The UI thread never blocks** on network or disk-heavy work.

## Testing conventions

- `cargo test` — 93 tests at handoff; keep it green and growing.
- Render tests use ratatui `TestBackend` (`tests/render.rs`): content
  assertions + ASCII purity. App logic is tested by driving real key
  events (`tests/editor_flow.rs`, `tests/options_flow.rs`) — that style
  caught a routing bug content tests missed; prefer it.
- Notification detection: `tests/notify_flow.rs` feeds `App::apply` with
  synthetic updates and inspects `test_take_pending()`.
- No hardware needed:
  ```sh
  cargo run --example mock_nis -- 3551 rack-main &
  cargo run --example mock_nis -- 3552 rack-aux onbatt &      # battery cycle
  cargo run --example mock_nis -- 3553 closet commlost &      # USB-unplug sim
  cargo run -- --ups a=127.0.0.1:3551 --ups b=127.0.0.1:3552
  ```
- Pushbullet end-to-end: point `APCTUI_PUSHBULLET_URL` at a local HTTP
  server that logs POSTs; assert request shape (`Access-Token` header,
  `{"type":"note",...}` body) and count — duplicates have happened.
- Real-hardware checks the owner can run: pull wall power (ONBATT within
  ~4s), pull the USB data cable (COMMLOST — but apcupsd's default POLLTIME
  is 60s and our generated confs don't set it, so allow ~2 minutes).

## Git workflow

- Never commit to `main`. Branch per change (`fix/...`, `feat/...`,
  `docs/...`), PR via `gh pr create --fill --web`, owner reviews and
  merges in the web UI, delete branches after merge.
- Commit messages: conventional-ish prefix, body explains the why and
  states how it was verified. Read `git log` for the house style.
- When behavior changes, update README in the same commit. Screenshots
  live in `docs/screenshots/` and are real captures, not mockups.
- Version bumps + `git tag vX.Y.Z` at meaningful milestones; the embedded
  git hash covers everything between.

## Known backlog (owner-approved directions, not yet built)

- LOWBATT notification — detection plumbing makes it a ~20-line addition
  to `App::apply` + an options toggle.
- Unsaved-changes prompt for the config editor (`c`) — same pattern as the
  options prompt, but per-tab saves go through pkexec + daemon restart, so
  "save all & close" needs its own design pass.
- Generated master configs don't set POLLTIME (60s default) — worth
  surfacing or setting, given it gates COMMLOST detection latency.
- `provider` in `[notifications]` is an enum of one; structure anticipates
  more services.
- Notification lock is per-machine, not per-fleet — two hosts watching the
  same units will both send. Documented, unsolved.

## Hard-learned lessons (cheap for you, expensive for us)

- A "working" test button doesn't mean the feature is armed. Token saved
  with the master switch off produced silent nothing; the guardrails that
  fixed it (header indicator, loud toasts, save prompts) are the product's
  personality now. Extend that philosophy.
- Stale binaries waste whole debugging sessions. `apctui --version` hash,
  always, before believing a repro.
- Scripted multi-site edits must assert their anchors exist. A silent
  no-op replace once nested a key-routing guard inside the wrong block and
  shipped a bug where typing "q" in a text field quit the view.
- Duplicate notifications came from two running instances, not the code.
  When a bug won't reproduce, suspect the environment before the diff.
- Tests that mutate a process-global env var (`XDG_CONFIG_HOME`, read by
  `options::config_path` and the notifier lock path) race under default
  parallel `cargo test` — green single-threaded, flaky in the suite. Fix:
  a poison-tolerant `static Mutex` around the set/use/restore window and a
  unique temp dir per call (an atomic counter, NOT bare `process::id()` —
  that's constant across the binary). Helpers: `with_temp_home`
  (src/options.rs), `with_xdg_home` (tests/options_flow.rs). Always run the
  suite parallel; single-threaded hides this whole class.
- flock release after closing the fd is NOT instantly visible to an
  immediate re-acquire under load (~27% miss in the full suite). Never
  assert instant lock takeover. Tests poll briefly; `App::options_save`
  reclaims its own just-released lock with a bounded ~50ms retry so re-arm
  is deterministic instead of stranding in standby until tick()'s 10s
  takeover. The 10s takeover for *another* instance's lock stays as-is —
  that one is correctly eventual.
