#!/usr/bin/env python3
import hid
import time
import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from datetime import datetime

VENDOR_ID = 0x3710
PRODUCT_ID = 0x5406
PORT = 3131

def checksum(*values):
    return (0x55 - sum(values)) & 0xFF

def build_hid_payload(command, index04=0x00, index05=0x00, index06=0x00):
    data = [
        0x08, command, 0x00, 0x00, index04, index05, index06,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
    ]
    data.append(checksum(*data))
    return data

def get_device_path():
    for dev in hid.enumerate(VENDOR_ID, PRODUCT_ID):
        if dev['interface_number'] == 1:
            return dev['path']
    return None

PollingRateHz = {0x01: 1000, 0x02: 500, 0x04: 250, 0x08: 125}

# Global state to hold the latest mouse info
mouse_state = {
    "connected": False,
    "power": None,
    "settings": {},
    "profile": None,
    "last_update": None,
    "error": None
}

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
        self.device.set_nonblocking(1)
        while True:
            res = self.device.read(17)
            if not res:
                break
        self.device.set_nonblocking(0)

    def write_read(self, payload, expect_cmd):
        self.clear_queue()
        self.device.write(payload)
        
        start_time = time.time()
        while (time.time() - start_time) < 1.0:
            resp = self.device.read(17, timeout_ms=100)
            if resp and len(resp) >= 2:
                if resp[0] == 0x08 and resp[1] == expect_cmd:
                    return resp
                elif len(resp) >= 1 and resp[0] == expect_cmd:
                    return [0x08] + resp
        return None

    def get_power(self):
        payload = [0x08, 0x04] + [0x00]*14 + [0x49]
        self.device.set_nonblocking(1)
        # flush queue
        while self.device.read(17): pass
        self.device.set_nonblocking(0)
        
        self.device.write(payload)
        
        start_time = time.time()
        while (time.time() - start_time) < 1.0:
            resp = self.device.read(17, timeout_ms=100)
            if resp and len(resp) >= 10:
                if resp[0] == 0x08 and resp[1] == 0x04:
                    return {
                        'percent': resp[6],
                        'charging': resp[7] == 1,
                        'voltage': (resp[8] << 8) | resp[9]
                    }
        print("Failed to get power response in get_power()")
        return None

    def read_settings(self):
        min_addr = 0x00
        max_addr = 0xb8
        current = min_addr
        while current <= (max_addr + 10):
            length = 10
            payload = build_hid_payload(0x08, index04=current, index05=length)
            resp = self.write_read(payload, 0x08)
            if resp:
                for i, v in enumerate(resp[6:6+length]):
                    self.settings[current + i] = v
            current += 10

    def get_active_profile(self):
        payload = build_hid_payload(0x0e)
        resp = self.write_read(payload, 0x0e)
        if resp:
            return resp[6]
        return None

def poll_mouse():
    """Background thread that constantly updates the global mouse_state"""
    global mouse_state
    while True:
        try:
            mouse = PulsarDevice()
            mouse_state["connected"] = True
            mouse_state["error"] = None
            
            # Read full settings once per connection to save USB traffic
            # or we can read it periodically. Let's do it on every successful poll to keep it fresh
            mouse.read_settings()
            mouse_state["settings"] = mouse.settings
            mouse_state["profile"] = mouse.get_active_profile()
            
            while True:
                power = mouse.get_power()
                if power is None:
                    print("Mouse went to sleep or didn't respond, restarting connection loop...")
                    break # Break out to re-init device or retry
                
                mouse_state["power"] = power
                mouse_state["last_update"] = datetime.now().isoformat()
                
                # Sleep between polls
                time.sleep(10)
                
        except Exception as e:
            import traceback
            traceback.print_exc()
            mouse_state["connected"] = False
            mouse_state["error"] = str(e)
            time.sleep(5)
        finally:
            try:
                mouse.close()
            except:
                pass

class RequestHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        global mouse_state
        
        if self.path == '/waybar':
            self.send_response(200)
            self.send_header('Content-type', 'application/json')
            self.end_headers()
            
            if not mouse_state["connected"] or not mouse_state.get("power"):
                self.wfile.write(json.dumps({"text": "Disconnected", "class": "critical"}).encode())
                return
                
            power = mouse_state["power"]
            pct = power['percent']
            charging = "⚡ " if power['charging'] else ""
            
            status_class = "normal"
            if pct <= 15:
                status_class = "critical"
            elif pct <= 30:
                status_class = "warning"
            elif power['charging']:
                status_class = "charging"

            tooltip = f"Battery: {pct}%\nVoltage: {power['voltage']/1000:.3f}V\nStatus: {'Charging' if power['charging'] else 'Discharging'}\nLast Update: {mouse_state.get('last_update', 'Unknown')}"
            
            output = {
                "text": f"{charging}{pct}%",
                "tooltip": tooltip,
                "class": status_class,
                "percentage": pct
            }
            self.wfile.write(json.dumps(output).encode())
            
        elif self.path == '/' or self.path == '/index.html':
            self.send_response(200)
            self.send_header('Content-type', 'text/html')
            self.end_headers()
            
            html = """
            <!DOCTYPE html>
            <html>
            <head>
                <title>Pulsar X2 Dashboard</title>
                <style>
                    body { font-family: monospace; background: #1e1e1e; color: #d4d4d4; padding: 20px; }
                    .card { background: #252526; padding: 20px; border-radius: 8px; margin-bottom: 20px; box-shadow: 0 4px 6px rgba(0,0,0,0.3); }
                    h1 { color: #569cd6; }
                    h2 { color: #4ec9b0; border-bottom: 1px solid #333; padding-bottom: 5px; }
                    table { border-collapse: collapse; width: 100%; max-width: 400px; }
                    td { padding: 5px 0; }
                    .key { color: #9cdcfe; font-weight: bold; width: 50%; }
                    .val { color: #ce9178; }
                    .critical { color: #f48771; font-weight: bold; }
                    .charging { color: #b5cea8; font-weight: bold; }
                </style>
                <script>
                    setTimeout(function(){ location.reload(); }, 5000);
                </script>
            </head>
            <body>
                <h1>Pulsar X2 Diagnostic Dashboard</h1>
            """
            
            if not mouse_state["connected"]:
                html += f"<div class='card'><h2 class='critical'>Device Disconnected</h2><p>Error: {mouse_state['error']}</p></div>"
            else:
                p = mouse_state.get("power", {})
                s = mouse_state.get("settings", {})
                
                if p:
                    status = "Charging" if p.get('charging') else "Discharging"
                    status_css = "charging" if p.get('charging') else "val"
                    html += f"""
                    <div class='card'>
                        <h2>Power Status</h2>
                        <table>
                            <tr><td class='key'>Battery:</td><td class='val'>{p.get('percent')}%</td></tr>
                            <tr><td class='key'>Voltage:</td><td class='val'>{p.get('voltage', 0)/1000.0:.3f} V</td></tr>
                            <tr><td class='key'>State:</td><td class='{status_css}'>{status}</td></tr>
                            <tr><td class='key'>Last Update:</td><td class='val'>{mouse_state.get('last_update', '')}</td></tr>
                        </table>
                    </div>
                    """
                
                if s:
                    polling_raw = s.get(0x00, 0)
                    polling_hz = PollingRateHz.get(polling_raw, "Unknown")
                    dpi_mode = s.get(0x04, 0)
                    dpi_mode_ct = s.get(0x02, 0)
                    lod = s.get(0x0a, 0)
                    lod_str = "1mm" if lod == 1 else "2mm" if lod == 2 else str(lod)
                    debounce = s.get(0xa9, 0)
                    sleep_time = s.get(0xb7, 0) * 10
                    motion_sync = "Enabled" if s.get(0xab, 0) else "Disabled"
                    angle_snap = "Enabled" if s.get(0xaf, 0) else "Disabled"
                    lod_ripple = "Enabled" if s.get(0xb1, 0) else "Disabled"
                    
                    led_effect_raw = s.get(0x4c, 0)
                    led_enabled = bool(s.get(0x52, 0))
                    led_effect = "Steady" if led_effect_raw == 1 else "Breathe" if led_effect_raw == 2 else "Unknown"
                    if not led_enabled:
                        led_effect = "Off"
                        
                    html += f"""
                    <div class='card'>
                        <h2>Hardware Settings</h2>
                        <table>
                            <tr><td class='key'>Active Profile:</td><td class='val'>{mouse_state.get('profile')}</td></tr>
                            <tr><td class='key'>Polling Rate:</td><td class='val'>{polling_hz} Hz</td></tr>
                            <tr><td class='key'>DPI Mode:</td><td class='val'>Slot {dpi_mode + 1} of {dpi_mode_ct}</td></tr>
                            <tr><td class='key'>Lift-off Dist:</td><td class='val'>{lod_str}</td></tr>
                            <tr><td class='key'>Debounce Time:</td><td class='val'>{debounce} ms</td></tr>
                            <tr><td class='key'>Auto-Sleep:</td><td class='val'>{sleep_time} seconds</td></tr>
                        </table>
                    </div>
                    <div class='card'>
                        <h2>Sensor Features</h2>
                        <table>
                            <tr><td class='key'>Motion Sync:</td><td class='val'>{motion_sync}</td></tr>
                            <tr><td class='key'>Angle Snapping:</td><td class='val'>{angle_snap}</td></tr>
                            <tr><td class='key'>LOD Ripple:</td><td class='val'>{lod_ripple}</td></tr>
                        </table>
                    </div>
                    <div class='card'>
                        <h2>Lighting</h2>
                        <table>
                            <tr><td class='key'>LED State:</td><td class='val'>{led_effect}</td></tr>
                        </table>
                    </div>
                    """
            
            html += """
            </body>
            </html>
            """
            self.wfile.write(html.encode())
        else:
            self.send_response(404)
            self.end_headers()

def run_server():
    server = HTTPServer(('127.0.0.1', PORT), RequestHandler)
    print(f"Starting server on http://127.0.0.1:{PORT}")
    server.serve_forever()

if __name__ == '__main__':
    # Start polling thread
    t = threading.Thread(target=poll_mouse, daemon=True)
    t.start()
    
    # Run HTTP server
    run_server()
