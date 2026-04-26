import hid
import time
import sys

def get_battery():
    VENDOR_ID = 0x3710
    PRODUCT_ID = 0x5406
    
    path = None
    for dev in hid.enumerate(VENDOR_ID, PRODUCT_ID):
        if dev['interface_number'] == 1:
            path = dev['path']
            break
            
    if not path:
        print("Device interface not found")
        sys.exit(1)
        
    device = hid.device()
    try:
        device.open_path(path)
        # For hidraw, device.set_nonblocking(0) is the default
        
        # Wake/Init sequence observed in PCAP
        payload_04 = [0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x49]
        device.write(payload_04)
        device.read(17, timeout_ms=50)
        
        payload_03 = [0x08, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x4a]
        device.write(payload_03)
        device.read(17, timeout_ms=50)

        # Send 0801 command (from frame 25399, which originally returned 51%)
        payload = [0x08, 0x01, 0x00, 0x00, 0x00, 0x08, 0x2b, 0x5a, 0x87, 0x82, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xb6]
        device.write(payload)
        
        time.sleep(0.05)
        
        resp = device.read(17, timeout_ms=1000)
        
        if len(resp) >= 10:
            # Check if the Report ID (0x08) is the first byte of the returned list
            if resp[0] == 0x08:
                battery = resp[9]
            else:
                battery = resp[8]
            print(f"Raw Response: {[hex(x) for x in resp]}")
            print(f"Battery: {battery}%")
        else:
            print(f"Failed to read battery. Raw response: {resp}")
            
    except OSError as e:
        print(f"HID Error: {e}")
        print("You might need to run this with sudo if udev rules are not set up.")
    finally:
        device.close()

if __name__ == '__main__':
    get_battery()