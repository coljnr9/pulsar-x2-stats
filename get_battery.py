import usb.core
import usb.util
import sys
import time

def get_battery():
    # Pulsar X2 8K Dongle
    dev = usb.core.find(idVendor=0x3710, idProduct=0x5406)
    if dev is None:
        print("Device not found")
        sys.exit(1)

    detached_interfaces = []
    
    # Detach all active interfaces
    for cfg in dev:
        for intf in cfg:
            if dev.is_kernel_driver_active(intf.bInterfaceNumber):
                try:
                    dev.detach_kernel_driver(intf.bInterfaceNumber)
                    detached_interfaces.append(intf.bInterfaceNumber)
                except usb.core.USBError as e:
                    print(f"Could not detach interface {intf.bInterfaceNumber}: {e}")

    try:
        usb.util.claim_interface(dev, 1)
        # The captured request payload
        payload = bytes([0x08, 0x01, 0x00, 0x00, 0x00, 0x08, 0x07, 0x82, 0xed, 0x89, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x45])
        
        # Send SET_REPORT (Host to Device)
        dev.ctrl_transfer(
            bmRequestType=0x21,
            bRequest=0x09,
            wValue=0x0208,
            wIndex=1,
            data_or_wLength=payload
        )

        time.sleep(0.05) # Give it a moment to process

        # Read the response from the IN Endpoint (0x82)
        response = dev.read(0x82, 17, timeout=1000)
        
        # The battery level is exactly at index 9
        battery = response[9]
        print(f"Battery: {battery}%")

    except usb.core.USBError as e:
        import traceback
        traceback.print_exc()
        print(f"USB Error: {e}")
        
    finally:
        # Release our claim and hand it back to the Linux kernel
        usb.util.release_interface(dev, 1)
        usb.util.dispose_resources(dev)
        for intf in detached_interfaces:
            try:
                dev.attach_kernel_driver(intf)
            except Exception as e:
                print(f"Could not reattach interface {intf}: {e}")

if __name__ == "__main__":
    get_battery()
