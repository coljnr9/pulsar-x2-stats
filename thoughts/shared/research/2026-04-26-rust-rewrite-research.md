---
date: 2026-04-26T00:00:00Z
git_commit: 15ed7b5cc1efc1558d0afb19481c2fa5adfdc7f9
branch: main
repository: pulsar-x2-re
topic: "We're going to rewrite this in Rust. So strap in, go research, figure out what we need to know to go do that."
tags: [research, codebase, hidapi, pyusb, server, protocol]
status: complete
last_updated: 2026-04-26
---

# Research: Rust Rewrite Preparation

## Research Question
We're going to rewrite this in Rust. So strap in, go research, figure out what we need to know to go do that.

## Summary
The `pulsar-x2-re` codebase consists of 9 Python scripts representing an evolution of reverse-engineered device communication with Pulsar mice (specifically the standard models and the newer 8K dongles). The project relies heavily on raw USB/HID communication to read device state (battery, voltage) and memory maps (DPI, polling rate, LED settings). It exposes this data through a local HTTP server (`pulsar_server.py`) serving both an HTML diagnostic dashboard and a JSON endpoint consumed by a simple client script (`waybar_proxy.py`) for the Waybar status bar. There are two primary connection paradigms in use: an older `PyUSB` approach requiring explicit Linux kernel driver detachment, and a newer `hidapi` approach via `/dev/hidraw`.

## Detailed Findings

### Device Communication & Connection Methods
- **PyUSB (`usb.core`) Implementation:**
  - Found in `pulsar.py:461-487` and `get_battery.py:23-28`.
  - Used for older Vendor/Product IDs (`0x3554:0xf508` wireless, `0x3554:0xf507` wired) in `pulsar.py` and the 8K Dongle (`0x3710:0x5406`) in `get_battery.py`.
  - Requires manually detaching the kernel driver from Interface 1 and claiming it.
  - Sends data using `ctrl_transfer` (Host-to-Device Set Report) on endpoint `0x82` / wValue `0x0208`.
- **`hidapi` Implementation:**
  - Found in `pulsar_server.py:38-46`, `pulsar_status.py:31-38`, `monitor_battery.py:11-16`, `get_battery_hid.py`.
  - Targets the 8K Dongle (`0x3710:0x5406`).
  - Iterates over HID devices and opens the device path where `interface_number == 1`.
  - Uses standard `.write()` and `.read()` blocking/non-blocking calls. This approach is much less intrusive on Linux as it binds natively via `hidraw`.

### Payload Protocol & Checksum
- **Structure:** All communication involves fixed-length 17-byte payloads.
  - `Byte 0`: Report ID Header (`0x08`).
  - `Byte 1`: Command ID.
  - `Bytes 2-15`: Data/Address indices.
  - `Byte 16`: Checksum (`(0x55 - sum(bytes 0..15)) & 0xFF`).
- **Checksum Logic:** Implemented identically across all scripts, e.g., `pulsar.py:456-457`, `pulsar_server.py:10-11`.
- **Quirk (17-byte vs 18-byte):** `pulsar_status.py:20` prepends an extra `0x08` byte to the 17-byte payload array (`[0x08] + data`) when writing to `hidapi`. In contrast, `pulsar_server.py:14-17` sends exactly 17 bytes starting with `0x08`. This discrepancy needs checking during the Rust rewrite regarding how `hidapi` handles Report IDs across platforms.

### Commands & Memory Map
- **Memory Addressing:** `pulsar.py:228-316` contains an extensive, hardcoded memory map mapping addresses like `0x00` (Polling Rate), `0x04` (DPI Mode), `0x4c` (LED Effect) to byte chunks. Memory is fetched 10 bytes at a time using the `0x08` (MEM_GET) command (`pulsar.py:596-613`).
- **Power Details (Command `0x04`):**
  - Sends `08 04 00 00 ... 49`.
  - Response parses: Battery % at index 6, Charging State (bool) at index 7, and Voltage (mV) as a 16-bit big-endian integer at indices 8-9. (See `pulsar_server.py:75-80`).
  - `get_battery.py:40-45` shows a legacy or alternative `0x01` command where battery % is extracted directly from index 8 or 9.
- **Other Identified Commands (`pulsar.py:13-22`):**
  - `0x0e` / `0x0f`: Active Profile Get/Set.
  - `0x03`: Status (Power on/off state).
  - `0x09`: Restore.
  - `0x0a`: Device Event (Used for polling).

### HTTP Server & UI Components
- **`pulsar_server.py`:**
  - Uses Python's built-in `http.server` running on `127.0.0.1:3131`.
  - Runs a background thread (`poll_mouse`) that continuously fetches data every 10 seconds and caches it in a global dictionary `mouse_state`.
  - `/waybar` Endpoint (`line 144`): Returns a JSON dictionary mapping battery levels to CSS status classes (`critical`, `warning`, `charging`, `normal`) and a formatted text/tooltip.
  - `/` or `/index.html` Endpoint (`line 173`): Server-side renders an HTML diagnostic dashboard, formatted with vanilla CSS inside `<style>` tags. Automatically reloads every 5 seconds.
- **`waybar_proxy.py`:**
  - Simple GET request client to fetch `/waybar` JSON using `urllib.request`. Contains hardcoded 1.0s timeouts and outputs static fallback JSON on error.
- **`pulsar_status.py` & `monitor_battery.py`:**
  - CLI implementations that clear the screen using ANSI escapes (`\033[2J\033[H`) and redraw ASCII diagnostic tables/logs.

### Reverse Engineering Context
- `find_battery_pattern.py` and `parse_sequence.py` were used to comb through USB packet captures (`/tmp/pulsar_in.txt`, `/tmp/pulsar_out.txt`). They document the heuristic of finding monotonically decreasing byte values (`44 -> 43 -> 42`) in payloads to deduce the location of the battery index. 

## Code References
- `pulsar.py:228-316` — Exhaustive memory map constants (polling rate, LED, DPI modes).
- `pulsar.py:461-487` — Core PyUSB `usb.core` device enumeration, interface claiming, and control transfers.
- `pulsar_server.py:38-46` — HID API enumeration filtering for `interface_number == 1`.
- `pulsar_server.py:75-80` — Explicit battery % and voltage mV extraction from the `0x04` command.
- `pulsar_server.py:102-132` — Background polling loop maintaining the global `mouse_state`.
- `get_battery_hid.py:24-34` — Documents the `0x04`, `0x03`, and `0x01` wake/init specific raw hex payloads.
- `pulsar_status.py:20` — The 18-byte hidapi array prepend discrepancy.

## Architecture Notes
- **State Management:** The server uses a polling thread dropping state into a global dictionary. There's no asynchronous event-driven architecture; it's purely synchronous polling with sleep delays.
- **Protocol Quirk:** The device seems finicky with sleep states. `get_battery_hid.py` uses consecutive writes (`0804`, then `0803`) to "wake up" or initialize the device before issuing the actual `0801` battery command. `pulsar_server.py:118` drops the connection and restarts the loop if a request fails, indicating flaky connection persistence.

## Open Questions
- Does `hidapi` in Rust (e.g., the `hidapi` crate) require prepending the Report ID (`0x08`) as a separate byte like `pulsar_status.py` does, or sending it as part of the data array like `pulsar_server.py`?
- Do we need to support both PyUSB (for older models) and `hidapi` (for the 8K Dongle) in the Rust rewrite, or just focus on the 8K Dongle (`0x3710:0x5406`) via `hidapi`?
- Should the new Rust daemon use D-Bus/MPRIS instead of an HTTP server on port 3131 to broadcast Waybar status?
