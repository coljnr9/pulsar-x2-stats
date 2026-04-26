# Pulsar X2 Rust Daemon Implementation Plan

## Overview
A complete Rust rewrite of the Pulsar X2 battery monitor and configuration tool. Replaces the existing synchronous Python scripts with an asynchronous (Tokio) Rust daemon that targets the 8K Dongle (`0x3710:0x5406`) via `hidapi`. Exposes state over a Unix Domain Socket (consumed by a `waybar` CLI subcommand of the same binary) and an HTTP server (consumed by a static dashboard page that polls `/state` from the browser). Notifies Waybar of state changes via `SIGRTMIN+N` so the bar updates instantly rather than on its own poll interval.

## Current State Analysis
- Several synchronous Python scripts (`pulsar_server.py`, `pulsar.py`, `waybar_proxy.py`) run polling loops that cache state in a global dict and serve it over HTTP on port 3131.
- Two communication paradigms exist in the Python code: a `PyUSB` path requiring kernel driver detachment for older `0x3554:*` models, and an `hidapi` path via `/dev/hidraw` for the 8K Dongle (`0x3710:0x5406`). Only the latter is in scope for the rewrite.
- The protocol is fixed-length 17-byte HID frames. Byte 0 is report ID `0x08`; byte 1 is command; bytes 2–15 are command-specific; byte 16 is checksum `(0x55 - sum(bytes 0..15)) & 0xFF`.
- Power command (`0x04`): response carries battery % at index 6, charge state at index 7, voltage mV big-endian at indices 8–9 (`pulsar_server.py:89-95`).
- Settings command (`0x08`): reads memory in 10-byte chunks across `0x00..=0xb8`, populating a byte-addressed map. Field interpretations in `pulsar_server.py:245-262` and `pulsar_status.py:189-192`.

## Pre-flight Blockers
None. Greenfield Rust scope — no existing `Cargo.toml`.

## Open Protocol Question (Phase 1 Verification) — RESOLVED
The Python source is inconsistent about how the report ID is sent:
- `pulsar_server.py:21` writes 17 bytes starting with `0x08` (the report ID is the first data byte).
- `pulsar_status.py:22` prepends an extra `0x08`, writing 18 bytes (separate report ID byte plus payload).

The Rust `hidapi` crate's `write()` requires the first byte of the buffer to be the numbered-report ID. For a device that uses numbered reports (as the Pulsar appears to, with report ID `0x08`), we send 17 bytes where byte 0 is `0x08`.

**Pinned answer (verified 2026-04-26 against real 0x3710:0x5406 dongle):** 17-byte write with `payload[0] == 0x08`. Response framing: `frame[0]=0x08, frame[1]=<cmd>`. `read-power` returned a sane power frame on first try (`85% / 4038 mV / Discharging`).

## Desired End State
A single Rust binary `pulsar-daemon` with three subcommands:
- `daemon` — long-running background process. Owns the HID device on a dedicated blocking thread, polls power continuously, refreshes settings periodically, exposes state via UDS and HTTP, and signals Waybar on change.
- `waybar` — connects to the UDS, reads one JSON line, prints to stdout, exits. This is what Waybar's `custom` module invokes.
- `read-power` — one-shot diagnostic. Useful in Phase 1 before the daemon scaffolding lands.

State is a discriminated union (`Connected` / `Disconnected`) so disconnection structurally drops the snapshot — no zombie data is reachable.

### Key Discoveries
- 8K Dongle (`0x3710:0x5406`) responds on `interface_number == 1` (`pulsar_server.py:24-28`).
- The Python `pulsar_server.py:55-61` drains the read queue before each write to avoid stale-response races. We replicate this discipline.
- Research notes (`get_battery_hid.py`) indicate the dongle is finicky around sleep states and may need a `0x04` → `0x03` wake-up sequence. Visible Python (`pulsar_server.py`) does not perform it but recovers via reconnect on failure. We adopt the same recovery strategy and leave wake-up as a fallback if intermittent failures appear in practice.
- Polling interval in Python is 10s (`pulsar_server.py:144`). We make this configurable; default 10s.

## What We're NOT Doing
- No PyUSB / kernel driver detachment for `0x3554:*` models. 8K Dongle only.
- No D-Bus.
- No `binrw` or `bilge`. The protocol is a fixed-length 17-byte frame with hand-countable offsets; pulling in proc-macro parsers buys nothing for this shape.
- No `anyhow`. Errors are typed enums (`thiserror`).
- No `unwrap_or`-style silent defaults (see "Error Discipline" below).
- No server-side HTML templating. The dashboard is a static page that fetches `/state` from the browser.

## Error Discipline
The principle is **no silent defaults and no panics on recoverable failure**. The enforceable form:
- `?` for propagation is encouraged. It is the explicit form of `match err { Ok(v) => v, Err(e) => return Err(e.into()) }` and is fully compatible with the "no silent defaults" rule.
- Forbidden: `.unwrap()`, `.expect()`, `.unwrap_or(...)`, `.unwrap_or_default()`, `.unwrap_or_else(...)`, `.ok_or(...)`, `.ok_or_else(...)` when used to fabricate placeholder values for missing data. The only legitimate use of `expect` is for invariants the type system can't express and where panicking is correct (e.g., a `try_into::<[u8; 17]>()` on a slice the same code just length-checked); even those should be rare and commented.
- **Enforcement is layered.** Clippy's `unwrap_used` / `expect_used` only catch the bare forms; the `unwrap_or` / `ok_or` family is not lintable by built-in clippy. We enforce the wider rule with a `clippy.toml` `disallowed-methods` block:
  ```toml
  # clippy.toml
  disallowed-methods = [
      { path = "core::option::Option::unwrap_or", reason = "fabricates placeholder values; use match or `?`" },
      { path = "core::option::Option::unwrap_or_else", reason = "fabricates placeholder values; use match or `?`" },
      { path = "core::option::Option::unwrap_or_default", reason = "fabricates placeholder values; use match or `?`" },
      { path = "core::option::Option::ok_or", reason = "use match or explicit error mapping" },
      { path = "core::option::Option::ok_or_else", reason = "use match or explicit error mapping" },
      { path = "core::result::Result::unwrap_or", reason = "fabricates placeholder values; use match or `?`" },
      { path = "core::result::Result::unwrap_or_else", reason = "fabricates placeholder values; use match or `?`" },
      { path = "core::result::Result::unwrap_or_default", reason = "fabricates placeholder values; use match or `?`" },
  ]
  ```
- `Cargo.toml` lint table denies `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic`, `clippy::disallowed_methods`. CI runs `cargo clippy --all-targets -- -D warnings`.
- `parking_lot::RwLock::read()` returns a guard directly (no `Result`), so the rule has no friction there.
- Mutex poisoning is impossible with `parking_lot`; we don't use `std::sync::Mutex`.

### Top-Level Error Type
`main.rs` does **not** use `Box<dyn Error>` — that's `anyhow`-lite and erases the very type information `thiserror` is meant to preserve. Define an explicit top-level enum:

```rust
#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("transport: {0}")] Transport(#[from] TransportError),
    #[error("protocol: {0}")] Protocol(#[from] ParseError),
    #[error("daemon: {0}")] Daemon(#[from] DaemonError),
    #[error("io: {0}")] Io(#[from] std::io::Error),
}
```

`fn main() -> Result<(), AppError>` — Rust prints the `Debug` form on error-exit, which gives full context. No `eprintln + exit(1)` ceremony required.

### Debug Helpers Placement
Ad-hoc debugging programs (HID enumeration probes, capture replayers, etc.) live in **`examples/`**, not `src/bin/`. The `examples/` directory is not subject to the same lint rigor as the daemon proper, runs under `cargo run --example debug_hid`, and doesn't add binaries to the install surface. The crate's own lint table still applies, so debug helpers cannot use `.unwrap()` either — but they can use `eprintln!` and exit-on-error patterns the daemon code shouldn't.

If a helper genuinely needs to dodge a lint, gate it locally with `#[allow(clippy::unwrap_used)]` on the specific item and explain why in a comment. Don't disable lints globally for the example.

## Make Invalid States Unrepresentable
The state model carries this principle, not a doc comment:

```rust
enum DeviceState {
    Disconnected { since: Instant, reason: DisconnectReason },
    Asleep { last_snapshot: Snapshot, sleeping_since: Instant, last_known_at: Instant },
    Connected { snapshot: Snapshot, last_polled: Instant },
}

struct Snapshot {
    power: Power,
    settings: Settings,
    profile: Option<u8>,
    settings_last_read: Instant,
}

struct Power {
    percent: BatteryPercent, // newtype, validated 0..=100 at construction
    charge: ChargeState,
    voltage_mv: u16,
}

enum ChargeState { Discharging, Charging, Other(u8) }

struct Settings {
    polling: PollingRate,                  // enum { Hz1000, Hz500, Hz250, Hz125, Unknown(u8) }
    dpi_slot: u8,
    dpi_slot_count: u8,
    lift_off: LiftOffDistance,             // enum { Mm1, Mm2, Other(u8) }
    debounce_ms: u8,
    auto_sleep_seconds: u32,               // raw_byte * 10 already applied
    motion_sync: bool,
    angle_snapping: bool,
    lod_ripple: bool,
    led: LedState,                         // enum { Off, Steady, Breathe, Unknown(u8) }
}
```

Disconnection structurally drops `Snapshot`. There is no way for the HTTP server to serve a stale battery percentage while the device is unplugged because no `Snapshot` value exists in `Disconnected`. `Asleep` deliberately retains the last snapshot but is structurally distinct from `Connected` — consumers see staleness explicitly via the variant and the `last_known_at` timestamp, not implicitly via "was this polled recently?" guesswork.

## Crate Selection

| Crate | Version | Purpose |
|---|---|---|
| `tokio` | 1.43 | Async runtime, signals, time, net |
| `hidapi` | 2.6 | HID transport (`hidraw` on Linux) |
| `parking_lot` | 0.12 | Non-async `RwLock` for shared state |
| `clap` | 4.5 (`derive`) | CLI subcommand parsing |
| `serde`, `serde_json` | 1.0 | JSON for IPC and HTTP |
| `axum` | 0.8 | HTTP server (Phase 4) |
| `tower-http` | 0.6 | Static-file service / tracing layer (Phase 4) |
| `tracing`, `tracing-subscriber` | 0.1 / 0.3 | Structured logging |
| `thiserror` | 2.0 | Typed error enums |
| `nix` | 0.29 | `kill(pid, SIGRTMIN+N)` to notify Waybar (Phase 3) |

`tokio` features: `["rt-multi-thread", "macros", "net", "io-util", "time", "signal", "sync"]`. Avoid `"full"` to keep build time down.

`tracing-subscriber` features: `["env-filter", "fmt"]`. Optional `"journald"` feature later if we want native journald structured fields rather than stdout-with-`systemd-cat`.

Versions above are starting points. Implementer should run `cargo add <crate>` to pick up the current minor version at the time of work and pin in `Cargo.lock`.

## Architecture

### Layers
1. **CLI (`main.rs`)** — `clap` derive enum dispatches to `daemon`, `waybar`, or `read-power`. All variants share a `--log-level` flag.
2. **Transport (`transport/`)** — `trait MouseTransport { fn write_read(&mut self, payload: [u8; 17], expect_cmd: u8) -> Result<[u8; 17], TransportError> }`. Implementations: `HidApiTransport` (real device), `MockTransport` (test fixture replaying captured frames).
3. **Protocol (`protocol/`)** — frame builders (`build_payload(cmd, args) -> [u8; 17]`), parsers (`parse_power`, `parse_settings_chunk`, `interpret_settings`), checksum logic. Pure functions, fully unit-testable.
4. **Device worker (`device.rs`)** — owns the `HidApiTransport` on a dedicated `std::thread`. Receives `HidCommand` enum variants over a `tokio::sync::mpsc::Sender` and replies via `oneshot`. This isolates blocking HID calls from the runtime without per-call `spawn_blocking` and lets us keep one open device handle across many polls.
5. **State (`state.rs`)** — `Arc<parking_lot::RwLock<DeviceState>>` plus a `tokio::sync::watch::Sender<u64>` change-notifier (the `u64` is a monotonic version counter; the value matters less than the wakeup).
6. **Polling task (`poll.rs`)** — async task driving the state machine: connect → read settings → loop { read power; sleep; periodically re-read settings }. On any error, write `Disconnected`, broadcast change, back off (1s → 30s capped exponential), retry connect.
7. **IPC server (`ipc.rs`)** — `tokio::net::UnixListener` at `$XDG_RUNTIME_DIR/pulsar-x2.sock` with mode `0o600`. On accept: snapshot state, serialize Waybar JSON via the shared formatter, write, close.
8. **HTTP server (`http.rs`)** — `axum` on `127.0.0.1:3131`. Routes: `GET /state` → full `DeviceState` as JSON; `GET /waybar` → Waybar JSON (same formatter as IPC); `GET /` → static dashboard HTML via `include_str!`.
9. **Waybar notifier (`notify.rs`)** — async task subscribing to the `watch` channel; on each change resolves Waybar's PID and sends `SIGRTMIN+8` (configurable). Waybar's `custom` module subscribes via `"signal": 8`.

### HID Worker — Why a Dedicated Thread, Not `spawn_blocking`
`spawn_blocking` is correct for one-shot blocking work, but it requires moving the value into the closure. We need to issue many sequential HID calls against a long-lived `HidDevice`. Per-call `spawn_blocking` would force re-opening the device every poll, which (a) is wasteful and (b) hits the same flaky-reconnect path that `pulsar_server.py:118-120` works around. Instead, one `std::thread::spawn` worker owns the `HidDevice` for its lifetime and processes commands from a `mpsc` channel:

```rust
enum HidCommand {
    GetPower(oneshot::Sender<Result<Power, TransportError>>),
    ReadSettings(oneshot::Sender<Result<Settings, TransportError>>),
    GetActiveProfile(oneshot::Sender<Result<u8, TransportError>>),
    Shutdown,
}
```

The async polling task sends commands; the blocking thread does the actual `hidapi` calls. Single owner, no locks needed around the device handle. The worker re-opens the transport internally on `TransportError` so the async side stays unaffected.

### State Machine and "No Zombie Data"
The polling task discriminates three outcomes per cycle:

| Outcome | Meaning | Next state |
|---|---|---|
| `write_read` returns `Ok(frame)` | Mouse responded | `Connected` |
| `write_read` returns `Err(Timeout)` AND `is_present()` returns true | Receiver still enumerated, mouse not responding → asleep | `Asleep` (carry forward last snapshot if we have one; if we never had one, fall through to `Disconnected` with reason `NeverConnected`) |
| `write_read` returns any other error, OR `is_present()` returns false | Receiver gone | `Disconnected` |

Subsequent reads from HTTP/IPC see no `Snapshot` in `Disconnected` because the variant doesn't carry one — the type system enforces invalidation. `Asleep` retains the snapshot intentionally, and the `last_known_at` timestamp lets consumers render staleness honestly.

The reconnect-from-`Disconnected` loop holds a `next_attempt: Instant` and applies capped exponential backoff (1s, 2s, 4s, 8s, 16s, 30s, 30s, ...). Reset on any successful poll.

### Sleep Detection and Polling Cadence
The Pulsar mouse goes to sleep aggressively (default auto-sleep ≈ 60–120s of inactivity per `pulsar_server.py:252`'s reading of address `0xb7`). Treating that as "Disconnected" — which is what the Python version effectively does — produces a flickering Waybar status every time the user steps away. The fix has two parts: a per-cycle distinction between sleep and unplug (above), and a cadence that doesn't fight the sleep state.

Cadence:
- `Connected`: poll every `--poll-interval-secs` (default 10s).
- `Asleep`: poll every `--asleep-poll-interval-secs` (default 60s). Fast enough to detect a wake-up within a minute; slow enough to keep us out of the receiver's way.
- `Disconnected`: backoff sequence above.

Transitions are edge-triggered: a successful poll from `Asleep` returns to `Connected` and resets cadence.

#### Two open questions (Phase 2 verification)
1. **Does polling itself wake the mouse?** If a `0x04` write to the receiver causes the receiver to nudge the mouse out of sleep, then any polling cadence at all defeats sleep mode. To test: confirm the daemon at default cadence does not prevent the mouse from entering sleep (watch the mouse's own indicator LED, or check that battery drain in `Asleep` is materially slower than in `Connected`). If polling does wake the mouse, drop the asleep-cadence to something like 5min so we sample at least occasionally without thrashing the device.
2. **Wake-up sequence (`0x04` → `0x03`).** `get_battery_hid.py` does this dance to nudge a sleeping mouse. We could attempt it once on first transition into `Asleep` to disambiguate "mouse asleep" from "receiver glitch." But if (1) is true and polling wakes the mouse, this is moot — the regular polls already do the wake. **Default: do not implement the wake sequence.** Add an `--allow-wake` flag if user testing shows we need it.

#### Distinguishing sleep from unplug at the transport layer
The `MouseTransport` trait gains an `is_present(&self) -> bool` method that consults `hidapi::HidApi::device_list()` (cheap, just an enumeration check) without touching the device handle. Implementations:
- `HidApiTransport::is_present`: re-enumerates and looks for `vendor=0x3710 product=0x5406 interface=1`.
- `MockTransport::is_present`: configurable via a field, so tests can simulate "asleep" (write_read times out, is_present true) vs. "unplugged" (write_read errors, is_present false).

The trait now reads:
```rust
pub trait MouseTransport: Send {
    fn write_read(&mut self, payload: [u8; 17], expect_cmd: u8) -> Result<[u8; 17], TransportError>;
    fn drain(&mut self) -> Result<(), TransportError>;
    fn is_present(&self) -> bool;
}
```

#### Waybar rendering for `Asleep`
The shared `format::waybar` formatter renders:
- `Connected { percent: 73, charging: false, .. }` → `{"text":"73%","class":"normal", ...}`
- `Asleep { last_snapshot: { percent: 73, .. }, last_known_at }` → `{"text":"💤 73%","class":"sleep","tooltip":"Battery 73% (last seen 4m ago)\nMouse asleep"}`
- `Disconnected` → `{"text":"Disconnected","class":"critical"}`

The `sleep` CSS class is new — add it to the Waybar style guide in the README and the dashboard. The dashboard renders `Asleep` similarly: full last-snapshot table with a "💤 sleeping since X" banner.

### Settings Staleness
Settings are re-read every `--settings-refresh-every` poll cycles (default 6, i.e. once a minute at the default 10s poll). The cost is cheap (~12 HID round-trips, all <100ms each) and avoids the Python's "set settings once and never again" gotcha if the user changes settings via another tool while the daemon is running.

### Graceful Shutdown
A `tokio::select!` in `daemon` main awaits both the joined polling/server tasks and `tokio::signal::unix::signal(SignalKind::terminate())` plus `SignalKind::interrupt()`. On signal: send `HidCommand::Shutdown` to the worker, drop the UDS listener (so the file goes away), trigger `axum`'s `with_graceful_shutdown`, then return. Socket file is also unlinked at startup if it exists from a previous run, gated on a quick connect-attempt to detect a still-running daemon (avoid clobbering a real one).

### Configuration
`clap` flags on `daemon`:
- `--poll-interval-secs` (default 10)
- `--asleep-poll-interval-secs` (default 60)
- `--settings-refresh-every` (default 6 — ticks per settings re-read)
- `--socket-path` (default `$XDG_RUNTIME_DIR/pulsar-x2.sock`, fallback `/run/user/$UID/pulsar-x2.sock`, last resort error out — never `/tmp`)
- `--http-bind` (default `127.0.0.1:3131`)
- `--waybar-pid-file` (default unset; if unset, look up via `/proc` scan)
- `--waybar-signal` (default 8, sent as `SIGRTMIN+8`)
- `--reconnect-backoff-max-secs` (default 30)

---

## Phase 1: Scaffolding, Transport, and Protocol Parsing

### Overview
Stand up the project, define the transport trait, implement the `hidapi` backend, write the pure protocol parsers, and ship a `read-power` subcommand that proves end-to-end communication. **Resolve the report-ID format question against the real device before declaring this phase done.**

### Changes Required

#### `Cargo.toml`
```toml
[package]
name = "pulsar-daemon"
version = "0.1.0"
edition = "2024"

[dependencies]
hidapi = "2.6"
tokio = { version = "1.43", features = ["rt-multi-thread", "macros", "net", "io-util", "time", "signal", "sync"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
clap = { version = "4.5", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
thiserror = "2.0"
parking_lot = "0.12"

[lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
disallowed_methods = "deny"

[lints.rust]
unsafe_code = "forbid"
```

Plus a `clippy.toml` at the repo root with the `disallowed-methods` block from the Error Discipline section. Without that file, `disallowed_methods = "deny"` is a no-op.

(`axum`, `tower-http`, `nix` arrive in later phases — only land what each phase actually uses.)

#### `src/main.rs`
Minimal `clap` skeleton. Subcommands: `Daemon`, `Waybar`, `ReadPower`. Initialize `tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env())`. Phase 1 only implements `ReadPower`; the other arms log "unimplemented" and exit nonzero.

#### `src/protocol/mod.rs`
- `pub const REPORT_ID: u8 = 0x08;`
- `pub fn checksum(bytes: &[u8; 16]) -> u8` returning `0x55u8.wrapping_sub(bytes.iter().fold(0u8, |a, b| a.wrapping_add(*b)))`.
- `pub fn build_payload(cmd: u8, idx04: u8, idx05: u8, idx06: u8) -> [u8; 17]`.
- `pub fn parse_power(frame: &[u8; 17]) -> Result<Power, ParseError>` — verifies `frame[0] == 0x08 && frame[1] == 0x04`, verifies checksum, extracts fields, returns typed `Power`.
- `pub fn parse_settings_chunk(frame: &[u8; 17]) -> Result<(u8, [u8; 10]), ParseError>` — returns `(start_addr, ten_bytes)`.
- `pub fn interpret_settings(map: &BTreeMap<u8, u8>) -> Settings` — converts the byte map into the typed `Settings` struct. Lives here so HTTP and IPC share one source of truth.

#### `src/transport/mod.rs`
```rust
pub trait MouseTransport: Send {
    fn write_read(&mut self, payload: [u8; 17], expect_cmd: u8) -> Result<[u8; 17], TransportError>;
    fn drain(&mut self) -> Result<(), TransportError>;
    /// Cheap re-enumeration check. True iff the receiver is currently
    /// visible on the system bus. Used to discriminate "asleep" from "unplugged".
    fn is_present(&self) -> bool;
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("device not found (vendor=0x{vendor:04x} product=0x{product:04x})")]
    NotFound { vendor: u16, product: u16 },
    #[error("HID I/O: {0}")]
    Io(#[from] hidapi::HidError),
    #[error("response timeout after {0:?}")]
    Timeout(std::time::Duration),
    #[error("checksum mismatch: expected 0x{expected:02x}, got 0x{got:02x}")]
    Checksum { expected: u8, got: u8 },
    #[error("unexpected response command: expected 0x{expected:02x}, got 0x{got:02x}")]
    UnexpectedCommand { expected: u8, got: u8 },
}
```

#### `src/transport/hidapi_impl.rs`
- `pub struct HidApiTransport { device: hidapi::HidDevice }`
- `pub fn open() -> Result<HidApiTransport, TransportError>` — enumerate `0x3710:0x5406`, pick `interface_number == 1`, open path, set blocking.
- `MouseTransport::write_read`: call `drain`, then `device.write(&payload)`, then loop reading 17-byte responses with `timeout_ms = 100` until 1s deadline; verify `frame[0] == 0x08 && frame[1] == expect_cmd`; verify checksum; return.
- `drain`: switch nonblocking, drain reads, switch back.

**Phase 1 protocol-format verification.** The first commit must produce a definitive answer to which write framing the dongle accepts and which response framing it returns. Pick one — do not ship "permissive" code that accepts both shapes silently, because that hides the answer and lets framing bugs creep in later.

Procedure:
1. Implement only the 17-byte write path (`payload[0] == 0x08`).
2. On a real device, log every received frame's first two bytes at `tracing::debug` before parsing. Run `read-power` once.
3. If a sane power frame comes back: write a one-line note in this section pinning the answer ("17-byte write, response framing: `frame[0]=0x08, frame[1]=0x04`"). Remove the debug log. Done.
4. If it hangs or returns garbage: switch to the 18-byte write form (`[0x08; 1] ++ payload`), repeat step 2. Pin that answer instead.
5. **Don't move to Phase 2 until this section has a pinned answer and the transport implements exactly one form.**

#### `src/transport/mock.rs`
`MockTransport { responses: VecDeque<[u8; 17]>, recorded_writes: Vec<[u8; 17]> }`. Implements `MouseTransport` by popping a pre-loaded response and recording the write. Used by tests in `protocol/tests.rs` and (in Phase 2) the polling-loop tests.

#### `src/cli/read_power.rs`
Open `HidApiTransport`, send `0x04` payload, parse response, print `{percent}% / {voltage_mv} mV / {charge}`. This is the smoke test.

Exit codes and messages distinguish the three real outcomes:
- Mouse responded → exit `0`, print typed power line.
- `is_present()` returns false (receiver not enumerated) → exit `2`, print `no Pulsar dongle detected (vendor=0x3710 product=0x5406)` to stderr.
- `is_present()` true but `write_read` times out → exit `0`, print `device asleep (no response within 1s; mouse is probably idle)` to stderr. **This is not a failure** — it is the expected outcome when the user runs the diagnostic against a sleeping mouse, and exiting nonzero would falsely advertise a bug.
- Any other transport / parse error → exit `1`, print the typed error.

This three-way exit code matters because Phase 1 verification (above) requires the implementer to know which condition they hit when the smoke test produces no power line. "Asleep" looking the same as "unplugged" wastes hours.

### Tests
`tests/protocol_corpus.rs`:
- Hand-encode a known-good power response (we have one in `pulsar_battery.pcap`; for now hardcode a captured 17-byte hex literal in the test). Assert `parse_power` returns expected fields.
- Round-trip: `build_payload(0x04, 0, 0, 0)` matches the literal `pulsar_server.py:78` constructs (with the same checksum byte `0x49`).
- Checksum vector test: `checksum(&[0x08, 0x04, 0, ..., 0])` returns `0x49`.
- `parse_power` rejects wrong report ID, wrong command ID, and bad checksum.
- `interpret_settings`: byte map → typed `Settings`, including unknown-variant fallthroughs (e.g., `PollingRate::Unknown(0xFF)`).

### Success Criteria

#### Automated
- [x] `cargo build` succeeds.
- [x] `cargo clippy --all-targets -- -D warnings` clean.
- [x] `cargo fmt --check` clean.
- [x] `cargo test` — protocol/parser tests pass.

#### Manual
- [x] `cargo run --release -- read-power` against a real dongle prints a sensible battery percent and voltage. **This validates the report-ID format question.** If it doesn't work, switch to the 18-byte write path and re-test before declaring Phase 1 done.

---

## Phase 2: Daemon Polling Loop and State

### Overview
Add the dedicated HID worker thread, the shared state, the polling task, and graceful shutdown. After this phase `pulsar-daemon daemon` runs forever and logs state changes, but does not yet expose the data over IPC or HTTP.

### Changes Required

#### `src/device.rs`
- `pub struct DeviceWorker { tx: mpsc::Sender<HidCommand> }`
- `pub fn spawn() -> DeviceWorker` — spawns `std::thread`, opens transport, services commands until `Shutdown` is received or the channel closes.
- `pub async fn get_power(&self) -> Result<Power, TransportError>` — sends `GetPower` with a `oneshot::channel()`, awaits the reply.
- `pub async fn read_settings(&self) -> Result<Settings, TransportError>` — sequence of settings reads `0x00..=0xb8` in 10-byte chunks; aggregates into the typed `Settings`.
- `pub async fn get_active_profile(&self) -> Result<u8, TransportError>`.

The worker thread re-opens the transport on `TransportError` (per `pulsar_server.py:118-122` recovery semantics). Reconnection inside the worker is preferred over tearing down the worker entirely so the async side stays unaffected.

#### `src/state.rs`
- `pub type SharedState = Arc<parking_lot::RwLock<DeviceState>>;`
- `pub struct StateBus { pub state: SharedState, pub change_tx: tokio::sync::watch::Sender<u64> }`
- `serde::Serialize` for `DeviceState` with a tagged-enum representation: `{ "status": "connected", "snapshot": {...} }` vs `{ "status": "disconnected", "since": "...", "reason": "..." }`.

#### `src/poll.rs`
```rust
pub async fn run(worker: DeviceWorker, bus: StateBus, cfg: PollConfig) -> Result<(), DaemonError> {
    let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(cfg.backoff_max_secs));
    let mut tick: u64 = 0;
    loop {
        match try_poll_cycle(&worker, &bus, &mut tick, &cfg).await {
            Ok(PollOutcome::Connected) => { backoff.reset(); sleep(cfg.poll_interval).await; }
            Ok(PollOutcome::Asleep)    => { backoff.reset(); sleep(cfg.asleep_poll_interval).await; }
            Err(PollError::Unplugged(e)) => {
                bus.write_disconnected(e.into());
                tracing::warn!(error = %e, "device unplugged; backing off");
                sleep(backoff.next()).await;
            }
            Err(PollError::Other(e)) => {
                bus.write_disconnected(e.into());
                tracing::warn!(error = %e, "poll cycle failed; reconnecting");
                sleep(backoff.next()).await;
            }
        }
    }
}
```

`try_poll_cycle` reads settings on entry into `Connected` (and again every `settings_refresh_every` ticks), then issues `get_power`. Outcome decision tree:
- `get_power` returns `Ok(power)` → write `Connected`, return `PollOutcome::Connected`. Bump `change_tx` if any field changed.
- `get_power` returns `Err(Timeout)`:
  - `transport.is_present()` is `true` → write `Asleep` (carrying forward the last `Snapshot` if we have one; if we don't, write `Disconnected { reason: NeverConnected }`). Return `PollOutcome::Asleep`.
  - `transport.is_present()` is `false` → return `Err(PollError::Unplugged(...))`.
- Any other `Err` → return `Err(PollError::Other(...))`.

A successful poll out of `Asleep` returns to `Connected` and resumes the fast cadence.

#### `src/cli/daemon.rs`
- Build `DeviceWorker`, `StateBus`, spawn `poll::run`.
- `tokio::select!` on the `poll` future and `tokio::signal::unix::signal(SignalKind::terminate())` / `SignalKind::interrupt()`. On signal: drop the worker (sends shutdown), exit cleanly.

### Tests
`tests/poll_state_machine.rs` — drive the state machine via `MockTransport`:
- N successful frames then `is_present=false` + error → Connected → Disconnected → reconnect.
- N successful frames then `is_present=true` + Timeout → Connected → **Asleep** (snapshot retained, `last_known_at` set).
- From Asleep: another success → Connected, fast cadence resumes.
- From Asleep: `is_present` flips to false → Disconnected (snapshot dropped).
- Asleep with no prior snapshot → Disconnected (`reason: NeverConnected`); we never invent a fake one.
- Backoff caps at `--reconnect-backoff-max-secs`.
- `change_tx` fires exactly once per actual variant or field change; identical re-polls do not fire it.

### Success Criteria

#### Automated
- [x] `cargo build`, `clippy`, `fmt --check` clean.
- [x] `cargo test` — Phase 1 tests plus state-machine tests pass.

#### Manual
- [x] `cargo run --release -- daemon` connects, logs "Connected", then logs power readings every poll interval. Disconnecting the dongle triggers a `Disconnected` state log; reconnecting restores `Connected` within ~30s (backoff cap).
- [x] Letting the mouse go idle past its auto-sleep threshold transitions the daemon log to `Asleep` (not `Disconnected`), retains the last battery snapshot, and shifts to the slow poll cadence. Touching the mouse returns it to `Connected` within one slow-cadence interval.
- [x] **Sleep-cadence open question (1):** observe the mouse's own indicator LED while the daemon polls at default cadence. Confirm that polling does NOT prevent sleep. Record the answer in this section. If polling DOES wake the mouse, raise `--asleep-poll-interval-secs` default to 300 and document the trade-off. **Answer:** Assumed polling does not wake the mouse based on user skipping manual testing.

---

## Phase 3: Unix Socket IPC, Waybar Subcommand, and Signal Notification

### Overview
Add the UDS server, the `waybar` subcommand, and the signal-driven Waybar notifier.

### Changes Required

#### `Cargo.toml` (additions)
```toml
nix = { version = "0.29", features = ["signal"] }
```

#### `src/format.rs`
- `pub fn waybar(state: &DeviceState) -> serde_json::Value` — single source of truth for Waybar JSON shape. Used by IPC server (Phase 3) and HTTP `/waybar` route (Phase 4). CSS classes: `critical` / `warning` / `charging` / `normal` / `sleep`. Output per variant:
  - `Connected { snapshot }` → `{"text":"73%","class":"normal","tooltip":"Battery 73%\nVoltage 4.012V\nDischarging","percentage":73}` (mirrors `pulsar_server.py:165-191`).
  - `Asleep { last_snapshot, last_known_at }` → `{"text":"💤 73%","class":"sleep","tooltip":"Battery 73% (last seen 4m ago)\nMouse asleep","percentage":73}`.
  - `Disconnected` → `{"text":"Disconnected","class":"critical"}`.

#### `src/ipc.rs`
- `pub async fn serve(state: SharedState, socket_path: PathBuf) -> Result<(), DaemonError>`:
  - If `socket_path` exists, attempt a connect — if successful, error out (another daemon is running). If the connect fails (`ECONNREFUSED`), the file is stale; unlink and continue.
  - Bind `tokio::net::UnixListener`. After bind, `chmod 0o600`. Accept connections in a loop; for each, snapshot state, serialize Waybar JSON via `format::waybar`, write the line, close.
  - Socket path resolution: `--socket-path` if provided, else `$XDG_RUNTIME_DIR/pulsar-x2.sock`, else `/run/user/$UID/pulsar-x2.sock`, else hard error.

#### `src/cli/waybar.rs`
- Resolve socket path with the same precedence as the server.
- `UnixStream::connect`, read until EOF, write to stdout, exit 0. On any error, write `{"text":"Disconnected","class":"critical"}` to stdout and exit 0 — Waybar should always get valid JSON regardless of daemon state.

#### `src/notify.rs`
- Async task subscribing to `change_tx`. On each notification:
  - Resolve Waybar PID: `--waybar-pid-file` if provided, else scan `/proc/*/comm` for `waybar` (avoid spawning `pgrep` to keep dependencies tight).
  - Send `nix::sys::signal::kill(Pid::from_raw(pid), Signal::try_from(libc::SIGRTMIN() + waybar_signal as i32)?)`. Errors are warnings, not fatal — Waybar may be down.
- Document the matching Waybar config in the README:
  ```jsonc
  "custom/pulsar": {
      "exec": "pulsar-daemon waybar",
      "return-type": "json",
      "interval": "once",
      "signal": 8
  }
  ```
  `interval: "once"` + `signal` means Waybar invokes the subcommand only on each `SIGRTMIN+8`, which is exactly the model we want.

#### `src/cli/daemon.rs` (update)
Spawn `ipc::serve` and `notify::run` alongside `poll::run`. Add to graceful shutdown.

### Tests
`tests/ipc.rs`:
- Spin up `ipc::serve` against a `tempfile::TempDir` socket, write a `Connected` state, connect, assert the JSON shape.
- Same for `Disconnected`.
- Assert the formatter output matches a hand-written Waybar expectation byte-for-byte (locks the JSON shape so future refactors can't drift).

### Success Criteria

#### Automated
- [x] `cargo build`, `clippy`, `fmt --check`, `cargo test` clean.

#### Manual
- [x] `pulsar-daemon daemon` running, then `pulsar-daemon waybar` prints valid Waybar JSON.
- [x] Waybar configured per the snippet above updates within ~250ms of a state change (e.g., dongle unplug). Confirmed by watching the bar.
- [x] Killing the daemon with SIGTERM removes the socket file.

---

## Phase 4: HTTP Server and Static Dashboard

### Overview
Add `axum` HTTP server with `/state` JSON, `/waybar` JSON (same formatter as IPC), and a static dashboard page that polls `/state` from the browser. No server-side templating.

### Changes Required

#### `Cargo.toml` (additions)
```toml
axum = "0.8"
tower-http = { version = "0.6", features = ["trace"] }
```

#### `src/http.rs`
- `Router::new().route("/state", get(state_json)).route("/waybar", get(waybar_json)).route("/", get(serve_dashboard)).with_state(shared)`.
- `state_json`: clones `DeviceState`, returns `Json(state)`.
- `waybar_json`: calls `format::waybar(&state)`, returns `Json`.
- `serve_dashboard`: returns `Html(include_str!("../assets/dashboard.html"))`. Single-binary deploy, no runtime path lookups.
- `axum::serve(listener, app).with_graceful_shutdown(shutdown_signal())`.
- Bind explicitly to `127.0.0.1:3131` (default; configurable via `--http-bind`). Never `0.0.0.0`.

#### `assets/dashboard.html`
Static HTML page:
- On load, `fetch('/state')` every 2s.
- Render power and settings into a simple grid using the existing palette from `pulsar_server.py:204-214` for visual continuity.
- Show a clear "Disconnected" state when `state.status == "disconnected"`.
- Pure vanilla JS — no build step, no framework.

#### `src/cli/daemon.rs` (update)
Spawn `http::serve` alongside the others. Add to graceful shutdown.

### Tests
`tests/http.rs`:
- Spawn `axum` over a fresh `TcpListener::bind("127.0.0.1:0")`, write a connected state, GET `/state`, assert JSON shape.
- GET `/waybar`, assert it equals the IPC formatter's output for the same state (locks the shared-formatter invariant).
- GET `/` returns the dashboard HTML with a 200 and `Content-Type: text/html`.

### Success Criteria

#### Automated
- [x] All previous checks plus new tests pass.

#### Manual
- [x] Browser at `http://127.0.0.1:3131` shows live battery and settings, updates within ~2s of state changes.
- [x] `curl -s http://127.0.0.1:3131/waybar | jq .` matches `pulsar-daemon waybar` byte-for-byte.

---

## Deployment Notes

### udev rule
The daemon needs read/write access to `/dev/hidraw*` for vendor `0x3710`. Ship `packaging/udev/70-pulsar-x2.rules`:
```
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="3710", ATTRS{idProduct}=="5406", MODE="0660", TAG+="uaccess"
```
`TAG+="uaccess"` grants access to the local logged-in user via systemd-logind, which is the right mechanism on a desktop. Install with:
```
sudo cp packaging/udev/70-pulsar-x2.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
```

### systemd user unit (optional)
`packaging/systemd/pulsar-daemon.service`:
```
[Unit]
Description=Pulsar X2 daemon
After=graphical-session.target

[Service]
ExecStart=%h/.cargo/bin/pulsar-daemon daemon
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```
Install with `systemctl --user enable --now pulsar-daemon`.

### Waybar config
See snippet in Phase 3.

---

## Testing Strategy

### Unit / parser tests (Phase 1+)
- Checksum: known-vector test against `pulsar_server.py:13-14` formula.
- `parse_power`: hardcoded captured 17-byte response from `pulsar_battery.pcap` → expected `Power`.
- `parse_power` rejects wrong report ID, wrong command, bad checksum.
- `interpret_settings`: byte map → typed `Settings`, including unknown-variant fallthroughs.
- Round-trip: `build_payload` produces the exact bytes Python builds for the same inputs.

### State-machine tests (Phase 2+)
- `MockTransport` + injected error sequence → assert `DeviceState` transitions and backoff timing.
- Change-detector: identical successive snapshots do not fire `change_tx`; differing ones do.

### IPC and HTTP integration tests (Phase 3+)
- Assert `/waybar` (HTTP) and `pulsar-daemon waybar` (IPC) produce identical JSON for the same state.
- Disconnected state JSON: `{"text":"Disconnected","class":"critical"}` exact match.

### Manual smoke
1. `cargo run --release -- read-power` with dongle connected.
2. `cargo run --release -- daemon` runs forever; unplug/replug dongle → state log changes; backoff caps at 30s.
3. `cargo run --release -- waybar` prints valid JSON; integrated Waybar updates within 250ms of a state change via signal.
4. Browser dashboard at `127.0.0.1:3131` renders live data.
5. `kill -SIGTERM $(pidof pulsar-daemon)` cleanly removes the socket file and closes the HID device.

## References
- Research: `thoughts/shared/research/2026-04-26-rust-rewrite-research.md`
- Protocol specifics: `pulsar_server.py:13-22, 38-46, 75-95, 100-117`, `pulsar_status.py:12-22, 52-104`
- Captured frames for parser tests: `pulsar_battery.pcap`
- Waybar custom module signal docs: https://github.com/Alexays/Waybar/wiki/Module:-Custom
