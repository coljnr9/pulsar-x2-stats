import hid
import time
import sys
import json
import struct
from datetime import datetime

VENDOR_ID = 0x3710
PRODUCT_ID = 0x5406

def clear_screen():
    sys.stdout.write("\033[2J\033[H")
    sys.stdout.flush()

def get_device_path():
    for dev in hid.enumerate(VENDOR_ID, PRODUCT_ID):
        if dev['interface_number'] == 1:
            return dev['path']
    return None

def main():
    path = get_device_path()
    if not path:
        print("Pulsar 8K Dongle (Interface 1) not found!")
        sys.exit(1)

    device = hid.device()
    try:
        device.open_path(path)
    except OSError as e:
        print(f"Failed to open device: {e}")
        print("Make sure udev rules are applied or run with sudo.")
        sys.exit(1)

    # 0804 payload with checksum 0x49
    payload = [0x08, 0x04] + [0x00]*14 + [0x49]

    log_file = "/home/cole/llm-workspace/pulsar-x2-re/battery_log.jsonl"
    
    print(f"Starting Pulsar Battery Monitor... Logging to {log_file}")
    time.sleep(1)

    try:
        while True:
            # Send Request
            try:
                device.write(payload)
                time.sleep(0.05)
            except Exception as e:
                pass
                
            # Read until we find the battery response or time out
            start_time = time.time()
            valid_resp = None
            unexpected = []
            
            while (time.time() - start_time) < 1.0:
                try:
                    resp = device.read(17, timeout_ms=100)
                    if not resp:
                        continue
                    
                    if len(resp) >= 10 and resp[0] == 0x08 and resp[1] == 0x04:
                        valid_resp = resp
                        break
                    else:
                        unexpected.append([hex(x) for x in resp])
                except Exception as e:
                    break

            # Parse Response
            if valid_resp:
                resp = valid_resp
                # The data structure based on the python-pulsar-mouse-tool
                battery_pct = resp[6]
                is_charging = resp[7] == 1
                voltage_mv = (resp[8] << 8) | resp[9]
                
                status_text = "Charging" if is_charging else "Discharging"

                # Log to file
                log_entry = {
                    "timestamp": datetime.now().isoformat(),
                    "battery_percent": battery_pct,
                    "voltage_mv": voltage_mv,
                    "charging": is_charging
                }
                with open(log_file, "a") as f:
                    f.write(json.dumps(log_entry) + "\n")

                # Draw TUI
                clear_screen()
                print("=========================================")
                print("       PULSAR X2 BATTERY MONITOR         ")
                print("=========================================\n")
                print(f" Time:     {datetime.now().strftime('%H:%M:%S')}")
                print(f" Status:   {status_text}")
                print(f" Battery:  {battery_pct}%")
                print(f" Voltage:  {voltage_mv / 1000.0:.3f} V")
                print("\n=========================================")
                print(" Press Ctrl+C to exit")
            else:
                clear_screen()
                print("Waiting for valid response from mouse...")
                if unexpected:
                    print(f"Got {len(unexpected)} unexpected payloads.")
                    with open(log_file, "a") as f:
                        for u in unexpected:
                            f.write(json.dumps({
                                "timestamp": datetime.now().isoformat(),
                                "unexpected_payload": u
                            }) + "\n")

            time.sleep(2)

    except KeyboardInterrupt:
        clear_screen()
        print("Monitoring stopped.")
    finally:
        device.close()

if __name__ == "__main__":
    main()
