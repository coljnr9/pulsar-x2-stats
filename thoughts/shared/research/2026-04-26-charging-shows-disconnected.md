---
date: 2026-04-26T18:45:42Z
git_commit: 1ca43b5a20acb11daf4b28cd64a61627de2a76d2
branch: main
repository: pulsar-x2-re
topic: "When mouse is on a power-only charger (not USB-data to PC), Waybar shows Disconnected instead of Charging"
tags: [research, codebase, state-machine, poll, transport, charging]
status: complete
last_updated: 2026-04-26
---

# Research: Why Waybar shows "Disconnected" when the mouse is on a power-only charger

## Research Question
"When I plug my mouse in to charge, even though it's on a power-only connection (not connected through USB to my computer, just to a charger), I am seeing the disconnected state in my Waybar, when I'd expect it to show charging. What might be going on here?"

## Summary
The "charging" state in Waybar is **only** rendered when `DeviceState::Connected` carries a `Power` snapshot whose `charge` field equals `ChargeState::Charging`. That field is populated from byte 7 of a successful 17-byte HID power-response frame returned by the dongle (`src/protocol/mod.rs:128-144`). Reaching that state requires three things to happen on a single poll cycle:

1. The dongle (`0x3710:0x5406` interface 1) is enumerated on the host.
2. The dongle's wireless link to the mouse is up and the mouse responds to the `0x04` power command within the 1-second deadline in `HidApiTransport::write_read` (`src/transport/hidapi_impl.rs:53-83`).
3. The response frame passes report-ID, command, and checksum verification.

When the user plugs the mouse into a wall charger (no USB host), nothing in this codebase can directly observe charging-while-disconnected-from-the-PC: charging state is read by polling the **mouse** over the dongle's 2.4 GHz radio, not by inspecting any USB descriptor on the host. So the answer to "why disconnected" reduces to: which of those three preconditions is failing, and why does the daemon's state machine choose `Disconnected` rather than `Asleep` for that failure mode?

The state machine (`src/poll.rs:64-208`) has two distinct "non-responding mouse" branches and the routing is brittle:

- **Timeout + `is_present()` true + we were in `Connected` or `Asleep`** → transition to `Asleep` (renders `💤 N%`, class `sleep`).
- **Timeout + `is_present()` true + we were in `Disconnected`** → returns `PollError::Unplugged("NeverConnected")`, which the outer loop turns into `bus.write_disconnected(DisconnectReason::Unplugged)` and applies backoff. **We stay in Disconnected even though the dongle is enumerated.** (`src/poll.rs:176-178` + `src/poll.rs:226-228`.)
- **Any non-timeout transport error** (e.g., HID I/O, checksum mismatch, transient enumeration loss between `is_present()` calls) → `PollError::Unplugged` or `PollError::Other`, both routed to `bus.write_disconnected(...)`. There is no `Asleep` fallback for non-timeout errors. (`src/poll.rs:200-206`.)

There are several plausible reasons the user is seeing `Disconnected` rather than `Charging` (or `Asleep`) and the codebase does not currently distinguish between them. The most likely failure modes are detailed below.

## Detailed Findings

### 1. How "charging" is detected at all
The only path that sets `ChargeState::Charging` is `protocol::parse_power` reading byte 7 of a verified 17-byte response frame:

```rust
// src/protocol/mod.rs:128-144
pub fn parse_power(frame: &[u8; 17]) -> Result<Power, ParseError> {
    verify_frame(frame, cmd::POWER)?;
    let percent = BatteryPercent::new(frame[6]);
    let charge = match frame[7] {
        charge_code::DISCHARGING => ChargeState::Discharging,   // 0x00
        charge_code::CHARGING    => ChargeState::Charging,      // 0x01
        other => ChargeState::Other(other),
    };
    let voltage_mv = u16::from_be_bytes([frame[8], frame[9]]);
    Ok(Power { percent, charge, voltage_mv })
}
```

The frame originates from a successful round-trip to the **dongle** (`0x3710:0x5406`, interface 1) — `src/transport/hidapi_impl.rs:11-39`. The daemon never reads charge state from the host's USB stack; it has no notion of "the mouse is on USB power" except indirectly via what the mouse itself reports back over the wireless link.

This means: **for Waybar to show "Charging", the mouse must still be talking to the dongle while it's plugged into the charger.** If the firmware silences the wireless radio when on USB power (whether from a host or a dumb charger), the daemon will never see `ChargeState::Charging` regardless of how correct the host-side code is.

### 2. The Waybar formatter only renders Charging in the `Connected` branch
`src/format.rs:4-65` is the single source of truth for Waybar JSON. The variant matters:

- `DeviceState::Connected { snapshot, .. }` →
  - if `snapshot.power.charge == Charging` → class `"charging"`, text `"{percent}%"`.
  - else thresholded: `critical` ≤10, `warning` ≤20, `normal` otherwise.
- `DeviceState::Asleep { last_snapshot, last_known_at, .. }` →
  - `text: "💤 {percent}%"`, class `"sleep"`. **There is no charging branch for `Asleep`** — even if `last_snapshot.power.charge == Charging`, sleep wins.
- `DeviceState::Disconnected { .. }` →
  - `text: "Disconnected"`, class `"critical"`. **No percentage, no charging, period.**

So even if the mouse is genuinely charging but the dongle/mouse link is intermittent, the user can see "Disconnected" or "💤 N%" instead of a charging indicator depending on which transport error occurred and what state the daemon was in.

### 3. The state-machine's "Disconnected → Asleep" hole
This is the most surprising behavior in the file and the strongest candidate for the user's observation. From `src/poll.rs:159-198`:

```rust
Err(TransportError::Timeout(_)) => {
    if let Ok(true) = worker.is_present().await {
        let new_state = match &current_state {
            DeviceState::Connected { snapshot, .. } => DeviceState::Asleep { ... },
            DeviceState::Asleep { last_snapshot, sleeping_since, .. } => DeviceState::Asleep { ... },
            DeviceState::Disconnected { .. } => {
                return Err(PollError::Unplugged("NeverConnected".to_string()));
            }
        };
        ...
        Ok(PollOutcome::Asleep)
    } else {
        Err(PollError::Unplugged("Device not present".to_string()))
    }
}
```

And the outer loop at `src/poll.rs:226-228`:

```rust
Err(PollError::Unplugged(e)) => {
    bus.write_disconnected(DisconnectReason::Unplugged);
    ...
}
```

The trap: once the daemon enters `Disconnected`, **a timeout (with the dongle still enumerated) is treated as `Unplugged("NeverConnected")` and the daemon writes `Disconnected` again.** The daemon cannot transition out of `Disconnected` via a timeout, only via a successful poll. If the mouse is silent on the wireless link the entire time the daemon is started — which is exactly the situation when the user starts the daemon while the mouse is already on a charger — the daemon stays in `Disconnected` forever and Waybar shows "Disconnected".

The unit test at `tests/poll_state_machine.rs:183-211` deliberately encodes this exact behavior:

```rust
// 5. Asleep with no prior snapshot
let state = ... DeviceState::Disconnected { ... NeverConnected ... };
...
let outcome5 = try_poll_cycle(&worker, &bus, &mut tick, &cfg).await;
assert!(matches!(outcome5, Err(PollError::Unplugged(ref s)) if s == "NeverConnected"));
```

So the codebase asserts that this is the intended outcome — and the prior plan (`thoughts/shared/plans/2026-04-26-rust-rewrite.md:184`) phrases it as: *"if we never had one, fall through to `Disconnected` with reason `NeverConnected`"*. The motivation was honest staleness for the dashboard ("don't render a fake battery percentage we never observed") rather than UX for the charging case.

### 4. Non-timeout transport errors always go to Disconnected
`src/poll.rs:200-206`:

```rust
Err(e) => {
    if let Ok(true) = worker.is_present().await {
        Err(PollError::Other(e.to_string()))
    } else {
        Err(PollError::Unplugged(e.to_string()))
    }
}
```

Both `Other` and `Unplugged` route to `bus.write_disconnected(...)` (`src/poll.rs:226-235`). There is no Asleep fallback for non-timeout errors. The transport produces a non-timeout error in cases including:

- Any `hidapi::HidError` from `device.write(&payload)`, `device.read_timeout(...)`, or `device.set_blocking_mode(...)` — surfaced as `TransportError::Io` (`src/transport/hidapi_impl.rs:51, 62, 86, 94`).
- A 17-byte response with the right report ID and command but a bad checksum (`src/transport/hidapi_impl.rs:64-75`) → `TransportError::Checksum`.
- A response frame `parse_power` rejects after `write_read` already returned Ok — `TransportError::Protocol(...)` via `src/device.rs:109` and the `From<ParseError>` impl on `TransportError` (`src/transport/mod.rs:23`).

Compounding this, `src/device.rs:101-106` explicitly drops the cached `HidApiTransport` on any non-timeout error:

```rust
Err(e) => {
    if !matches!(e, TransportError::Timeout(_)) {
        *transport_opt = None;   // force reopen on next call
    }
    return Err(e);
}
```

So a single `EIO` or `ENODEV` blip on the hidraw fd causes the worker to drop the device, the next call goes through `HidApiTransport::open()` again, and the daemon flips to `Disconnected` for at least one cycle. If reopening succeeds and the next poll responds, the daemon recovers; if the mouse is silent, see point 3.

### 5. The dongle vs. mouse link is opaque to this codebase
`is_present()` only checks for `vendor=0x3710 product=0x5406 interface=1` in the host enumeration (`src/transport/hidapi_impl.rs:98-113`, mirrored in `src/device.rs:58-76`). It does not distinguish:

- Dongle present **and** wirelessly linked to a responsive mouse (the only case where `Connected` is reachable).
- Dongle present, mouse paired but not responding (sleep, charging-with-radio-off, or RF interference).
- Dongle present, mouse not paired at all (e.g., new dongle, or paired to a different unit).

There is no health-probe of the dongle itself — every reachability decision is inferred from the mouse's response to a power command.

### 6. Pulsar X2 firmware behavior on USB power (out of code; informs interpretation)
The Pulsar X2 (and most 2.4 GHz wireless gaming mice with a charging port) typically detects USB power on its own port. The exact firmware behavior — whether the wireless radio stays alive on a power-only connection, whether it switches to a wired-USB profile when the port is connected to a host, and whether the mouse reports `charge=0x01` over the wireless link while charging — is not documented in this repository. The legacy Python (`pulsar_server.py`, summarized in `thoughts/shared/research/2026-04-26-rust-rewrite-research.md:45-48`) parsed `charge` from byte 7 of the same frame, indicating the protocol can in principle return `Charging` over the wireless link. Whether the X2 8K dongle/mouse pair actually does so on a power-only USB connection is something a packet capture would settle; `pulsar_battery.pcap` exists in the tree but its contents have not been examined as part of this research.

### 7. The `pulsar_battery.pcap` capture
The repo carries `pulsar_battery.pcap` (5.5 MB) but no analysis tooling for it is in the current codebase. The earlier research note (`2026-04-26-rust-rewrite-research.md:67`) mentions `find_battery_pattern.py` and `parse_sequence.py` from the Python era, but those files are no longer in the tree (the rewrite commit replaced them). A capture of a charging event on the same model would be the cleanest way to confirm whether the mouse ever sends `frame[7] == 0x01` on a power-only USB connection.

## Code References

- `src/protocol/mod.rs:128-144` — `parse_power`; only path that produces `ChargeState::Charging`.
- `src/protocol/mod.rs:67-70` — `charge_code::CHARGING = 0x01`, `DISCHARGING = 0x00`.
- `src/format.rs:4-36` — `Connected` branch; only branch that emits class `"charging"`.
- `src/format.rs:37-57` — `Asleep` branch; ignores `last_snapshot.power.charge`.
- `src/format.rs:58-64` — `Disconnected` branch; emits `{"text":"Disconnected","class":"critical"}` unconditionally.
- `src/poll.rs:64-208` — `try_poll_cycle`; routes power-poll outcomes into state transitions.
- `src/poll.rs:159-198` — Timeout branch; the `Disconnected → NeverConnected` early return at `:176-178` is the loop trap.
- `src/poll.rs:200-206` — Non-timeout error branch; always routes to `Disconnected`.
- `src/poll.rs:226-235` — Outer-loop translation of `PollError` into `bus.write_disconnected(...)`.
- `src/state.rs:13-33` — `StateBus::write_disconnected`; only writes if reason changes.
- `src/transport/hidapi_impl.rs:42-83` — `write_read`; 1s deadline, 100ms read polls, inline report-ID/cmd/checksum verification.
- `src/transport/hidapi_impl.rs:77-80` — Frames with right report ID but wrong cmd are silently dropped (no log, no early return).
- `src/transport/hidapi_impl.rs:98-113` — `is_present`; pure host-side enumeration check, no link probe.
- `src/device.rs:101-106` — Worker invalidates the cached transport on any non-timeout error.
- `src/device.rs:58-76` — `handle_is_present`; same enumeration check used by the poll loop.
- `tests/poll_state_machine.rs:183-211` — Test that pins the `Disconnected + Timeout + present → still Disconnected` behavior.
- `packaging/waybar/custom-pulsar.json` — Waybar config: `interval: "once"` + `signal: 8`. Bar updates only when daemon sends `SIGRTMIN+8` via `src/notify.rs`.

## Architecture Notes

- **Discriminated state with no zombie data.** `DeviceState` is `Connected | Asleep | Disconnected` (`src/state.rs:35-55`). Only `Connected` and `Asleep` carry a `Snapshot`; `Disconnected` has no battery data by construction. This prevents the dashboard/waybar from rendering stale numbers but also means `Disconnected` cannot be styled as "charging" — there is no power data to attach to it.
- **Charge state is mouse-reported, not host-observed.** Nothing reads from `sysfs` (`/sys/class/power_supply`), the libinput device, or any USB-host-side hint about charging. The codebase fundamentally relies on the wireless link being up and the mouse choosing to respond.
- **Single-shot Waybar, signal-driven updates.** Waybar runs `pulsar-daemon waybar` once per `SIGRTMIN+8` (`packaging/waybar/custom-pulsar.json` + `src/notify.rs:14-37`). If the daemon is in `Disconnected` and never transitions out, Waybar will keep getting "Disconnected" from each signal. There is no fallback timer — Waybar's `interval: "once"` means polling is fully push-driven by the daemon.
- **Failure-mode coverage gap for Asleep on non-timeout errors.** The state machine treats only `TransportError::Timeout` as evidence of "mouse silent but reachable." Any I/O, checksum, or protocol error on the same fundamental cause (mouse not responding, possibly because charging state is doing something to the link) is conflated with "unplugged."
- **Worker reopen-on-error.** `device.rs:101-106` resets the cached `HidApiTransport` on every non-timeout error. This is conservative but couples HID quirks tightly to the user-visible state — a transient EIO bumps the daemon to `Disconnected` for at least one cycle.

## Historical Context (from thoughts/)

- `thoughts/shared/plans/2026-04-26-rust-rewrite.md:184` — Documents the "Disconnected with reason NeverConnected" choice for the no-prior-snapshot case as intentional, motivated by not wanting to fabricate a fake snapshot. Charging-while-on-charger was not in scope.
- `thoughts/shared/plans/2026-04-26-rust-rewrite.md:201-203` — Open questions section at plan time included "does polling itself wake the mouse" but did not address charging-link semantics.
- `thoughts/shared/plans/2026-04-26-rust-rewrite.md:419-422` — Decision tree for the `try_poll_cycle` outcomes; matches what shipped, including the `Asleep`-only-on-Timeout policy.
- `thoughts/shared/research/2026-04-26-rust-rewrite-research.md:45-48` — Original research note on the power command response shape: `frame[7]` carries charge state. Charging-via-wireless-link was therefore at least possible in principle in the Python predecessor.

## Related Research

- `thoughts/shared/research/2026-04-26-rust-rewrite-research.md` — original codebase research that informed the Rust rewrite.

## Open Questions

1. **Does the X2 8K mouse actually respond to dongle polls while it's on a power-only USB connection?** Answering this requires an out-of-tree experiment: with the dongle in the PC, attach the mouse to a wall charger only, and observe whether `pulsar-daemon read-power` returns `Charging` or hangs to timeout. Resolves whether the user is hitting the "mouse silent → state-machine routing" issue (sections 3/4) or the "firmware never reports charging on this path" issue (section 6).
2. **What concrete `TransportError` does `write_read` return when the mouse stops responding due to charging?** Pure timeout (Asleep path, with the Disconnected-trap caveat) vs. I/O error (always-Disconnected path) is decisive for which fix applies. A `tracing` capture at `RUST_LOG=debug` during the failure would localize this.
3. **At what state does the daemon enter the failure?** If the user starts the daemon with the mouse already on the charger, section 3 (Disconnected trap) is the simplest explanation: no prior snapshot → no transition out of Disconnected. If the failure happens after a period of normal use, section 4 (non-timeout error path) or a brief `is_present()`=false during an enumeration glitch is more likely.
4. **Does the dongle itself drop or re-enumerate when the mouse switches to a USB-power state?** A short `is_present()`=false window during the switch would route to `Disconnected{Unplugged}`, after which the daemon never recovers via timeouts (section 3). `udevadm monitor` while plugging in the charger would settle this.
5. **Is `pulsar_battery.pcap` a recording of a charging event?** If yes, replaying it through `parse_power` would show whether `frame[7]==0x01` ever appears. The file exists but has not been examined here.
