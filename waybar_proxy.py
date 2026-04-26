#!/usr/bin/env python3
import urllib.request
import json
import sys

try:
    with urllib.request.urlopen('http://127.0.0.1:3131/waybar', timeout=1.0) as response:
        if response.status == 200:
            data = response.read().decode('utf-8')
            print(data)
        else:
            print(json.dumps({"text": "Error", "class": "critical", "tooltip": "Server returned error"}))
except Exception as e:
    print(json.dumps({"text": "Disconnected", "class": "critical", "tooltip": f"Server unreachable: {e}"}))
