#!/usr/bin/env python3
import hid
import time
import sys
import json
import argparse
from datetime import datetime

VENDOR_ID = 0x3710
PRODUCT_ID = 0x5406

def checksum(*values):
    return (0x55 - sum(values)) & 0xFF

def build_hid_payload(command, index04=0x00, index05=0x00, index06=0x00):
    data = [
        0x08, command, 0x00, 0x00, index04, index05, index06,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
    ]
    data.append(checksum(*data))
    # Prepend Report ID 0x08 for hidapi
    return [0x08] + data

def get_device_path():
    for dev in hid.enumerate(VENDOR_ID, PRODUCT_ID):
        if dev['interface_number'] == 1:
            return dev['path']
    return None

class PulsarDevice:
    def __init__(self):
        self.path = get_device_path()
        if not self.path:
            raise Exception("Pulsar 8K Dongle not found.")
        self.device = hid.device()
        self.device.open_path(self.path)
        self.device.set_nonblocking(0)
        self.settings = {}

    def close(self):
        self.device.close()

    def clear_queue(self):
        # Drain any pending unexpected payloads
        self.device.set_nonblocking(1)
        while True:
            res = self.device.read(17)
            if not res:
                break
        self.device.set_nonblocking(0)

    def write_read(self, payload, expect_cmd):
        self.clear_queue()
        self.device.write(payload)
        
        # Try to read until we get the expected command response
        start_time = time.time()
        while (time.time() - start_time) < 1.0:
            resp = self.device.read(17, timeout_ms=100)
            if resp and len(resp) >= 2:
                # Sometimes hidapi returns report ID as first byte, sometimes not.
                # Usually resp[0] == 0x08 and resp[1] == command
                if resp[0] == 0x08 and resp[1] == expect_cmd:
                    return resp
                elif len(resp) >= 1 and resp[0] == expect_cmd:
                    return [0x08] + resp
        return None

    def get_power(self):
        # 08 04 command for battery/voltage
        payload = build_hid_payload(0x04)
        resp = self.write_read(payload, 0x04)
        if not resp:
            return None
        
        battery_pct = resp[6]
        is_charging = resp[7] == 1
        voltage_mv = (resp[8] << 8) | resp[9]
        return {
            'percent': battery_pct,
            'charging': is_charging,
            'voltage': voltage_mv
        }

    def read_settings(self):
        min_addr = 0x00
        max_addr = 0xb8
        current = min_addr
        while current <= (max_addr + 10):
            length = 10
            payload = build_hid_payload(0x08, index04=current, index05=length)
            resp = self.write_read(payload, 0x08)
            if resp:
                # resp[4] == current, resp[5] == length
                for i, v in enumerate(resp[6:6+length]):
                    self.settings[current + i] = v
            current += 10

    def get_active_profile(self):
        payload = build_hid_payload(0x0e)
        resp = self.write_read(payload, 0x0e)
        if resp:
            return resp[6]
        return None

def clear_screen():
    sys.stdout.write("\033[2J\033[H")
    sys.stdout.flush()

PollingRateHz = {0x01: 1000, 0x02: 500, 0x04: 250, 0x08: 125}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--waybar', action='store_true', help='Output JSON for Waybar')
    args = parser.parse_args()

    try:
        mouse = PulsarDevice()
    except Exception as e:
        if args.waybar:
            print(json.dumps({"text": "Disconnected", "class": "critical"}))
        else:
            print(str(e))
        sys.exit(0)

    try:
        if args.waybar:
            power = mouse.get_power()
            if power:
                pct = power['percent']
                charging = "⚡ " if power['charging'] else ""
                
                status_class = "normal"
                if pct <= 15:
                    status_class = "critical"
                elif pct <= 30:
                    status_class = "warning"
                elif power['charging']:
                    status_class = "charging"

                tooltip = f"Battery: {pct}%\nVoltage: {power['voltage']/1000:.3f}V\nStatus: {'Charging' if power['charging'] else 'Discharging'}"
                
                output = {
                    "text": f"{charging}{pct}%",
                    "tooltip": tooltip,
                    "class": status_class,
                    "percentage": pct
                }
                print(json.dumps(output))
            else:
                print(json.dumps({"text": "Error", "class": "critical"}))
        else:
            # TUI Mode
            print("Reading complete device memory map...")
            mouse.read_settings()
            power = mouse.get_power()
            profile = mouse.get_active_profile()
            
            clear_screen()
            print("=========================================")
            print("       PULSAR X2 DIAGNOSTIC DASHBOARD    ")
            print("=========================================\n")
            
            if power:
                status = "Charging" if power['charging'] else "Discharging"
                print(f" [ Power Status ]")
                print(f"   Battery:  {power['percent']}%")
                print(f"   Voltage:  {power['voltage']/1000:.3f} V")
                print(f"   State:    {status}\n")
            
            if mouse.settings:
                # Parse known addresses based on pulsar.py
                polling_raw = mouse.settings.get(0x00, 0)
                polling_hz = PollingRateHz.get(polling_raw, "Unknown")
                
                dpi_mode = mouse.settings.get(0x04, 0)
                dpi_mode_ct = mouse.settings.get(0x02, 0)
                
                lod = mouse.settings.get(0x0a, 0)
                lod_str = "1mm" if lod == 1 else "2mm" if lod == 2 else str(lod)
                
                debounce = mouse.settings.get(0xa9, 0)
                motion_sync = "Enabled" if mouse.settings.get(0xab, 0) else "Disabled"
                angle_snap = "Enabled" if mouse.settings.get(0xaf, 0) else "Disabled"
                lod_ripple = "Enabled" if mouse.settings.get(0xb1, 0) else "Disabled"
                sleep_time = mouse.settings.get(0xb7, 0) * 10
                
                led_effect_raw = mouse.settings.get(0x4c, 0)
                led_enabled = bool(mouse.settings.get(0x52, 0))
                led_effect = "Steady" if led_effect_raw == 1 else "Breathe" if led_effect_raw == 2 else "Unknown"
                if not led_enabled:
                    led_effect = "Off"

                print(f" [ Hardware Settings ]")
                print(f"   Active Profile: {profile}")
                print(f"   Polling Rate:   {polling_hz} Hz")
                print(f"   DPI Mode:       Slot {dpi_mode + 1} of {dpi_mode_ct}")
                print(f"   Lift-off Dist:  {lod_str}")
                print(f"   Debounce Time:  {debounce} ms")
                print(f"   Auto-Sleep:     {sleep_time} seconds\n")
                
                print(f" [ Sensor Features ]")
                print(f"   Motion Sync:    {motion_sync}")
                print(f"   Angle Snapping: {angle_snap}")
                print(f"   LOD Ripple:     {lod_ripple}\n")
                
                print(f" [ Lighting ]")
                print(f"   LED State:      {led_effect}")
                
            print("\n=========================================")

    finally:
        mouse.close()

if __name__ == '__main__':
    main()