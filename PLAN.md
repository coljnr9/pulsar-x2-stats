# Pulsar X2 USB Protocol Reverse Engineering Plan

This plan outlines the steps to capture, analyze, and reverse engineer the USB communication protocol for the Pulsar X2 mouse (specifically targeting battery life readings) using a Linux host and a Windows VM.

## Phase 1: Environment Setup

1.  **Windows Virtual Machine:** Ensure you have a working Windows VM (using QEMU/KVM with virt-manager or VirtualBox) installed on your Linux host.
2.  **Pulsar Software:** Install the official Pulsar configuration software inside the Windows VM.
3.  **USB Passthrough:** Configure the VM to take direct control of the Pulsar X2 mouse (or its wireless receiver) via USB passthrough.

## Phase 2: Capture Environment Preparation (Linux Host)

1.  **Load USB Monitor:** Load the `usbmon` kernel module on your Linux host to enable raw USB packet capture:
    ```bash
    sudo modprobe usbmon
    ```
2.  **Identify the Device:** Use `lsusb` on the host to find the Bus and Device number of the Pulsar X2 before passing it to the VM.
    *   *Example output: `Bus 003 Device 012: ID 258a:002a Pulsar X2...`*
    *   This tells us we need to listen on `usbmon3` (for Bus 003).
3.  **Wireshark Setup:** Ensure Wireshark is installed on the Linux host and your user has privileges to capture packets (e.g., added to the `wireshark` group).

## Phase 3: Traffic Capture

1.  **Start Capture:** Launch Wireshark on the Linux host and start capturing on the appropriate `usbmonX` interface.
2.  **Trigger Communication:**
    *   Start the Windows VM.
    *   Open the Pulsar software inside the VM.
    *   Wait for the software to detect the mouse and display the battery level.
    *   *Optional but recommended:* Perform a few distinct actions in the software (like changing DPI back and forth) and note the exact time you did them. This creates "landmarks" in the captured data to help isolate the battery requests.
3.  **Save Capture:** Stop the Wireshark capture and save the file (e.g., `pulsar_x2_capture.pcapng`).

## Phase 4: Protocol Analysis

1.  **Filter Traffic:** Open the capture file and apply a display filter to isolate only the traffic for the mouse:
    ```wireshark
    usb.bus_id == <BUS_NUMBER> && usb.device_address == <DEVICE_NUMBER>
    ```
2.  **Isolate HID Reports:** Most modern gaming mice use standard or vendor-specific HID (Human Interface Device) reports for configuration. We will look specifically for `URB_INTERRUPT` (in/out) or `URB_CONTROL` transfers.
3.  **Identify Battery Requests:** Analyze the packets sent from the host to the device just before the battery level is displayed in the app. We need to identify the exact byte sequence (the request) and the corresponding response from the mouse containing the battery percentage.

## Phase 5: Implementation (Proof of Concept)

1.  **Develop Python Script:** Once the request and response structure is understood, we will write a Python script using the `hidapi` or `pyusb` library.
2.  **Replicate Request:** The script will send the identified raw HID request to the mouse.
3.  **Parse Response:** The script will read the response from the mouse and extract the battery level byte, converting it to a human-readable percentage on your Linux host.
