import sys

def parse_hex(s):
    try:
        return bytes.fromhex(s)
    except:
        return b''

payloads = []
with open('/tmp/pulsar_in.txt', 'r') as f:
    for line in f:
        parts = line.strip().split()
        if len(parts) >= 2:
            frame_num = int(parts[0])
            for p in parts[1:]:
                data = parse_hex(p)
                if data:
                    payloads.append((frame_num, data))
                    break

print(f"Loaded {len(payloads)} payloads.")

# We are looking for any byte index in any payload length that transitions 44 -> 43 -> 42 or 43 -> 42
# Or generally, just monotonically decreasing values in the range [0, 100].

# Track history per (payload_length, byte_index) -> list of (frame_number, value)
history = {}

for frame_num, data in payloads:
    L = len(data)
    for i, val in enumerate(data):
        key = (L, i)
        if key not in history:
            history[key] = []
        
        # Only record if the value changed, to compress history
        if not history[key] or history[key][-1][1] != val:
            history[key].append((frame_num, val))

candidates = []
for key, changes in history.items():
    L, i = key
    
    # We want bytes that decrease and hit values around 50->40.
    # Check if this index ever contained 0x2A (42), 0x2B (43), or 0x2C (44)
    # and generally decreases.
    
    vals = [c[1] for c in changes]
    
    # Filter out static bytes (len(changes) == 1)
    if len(vals) < 2:
        continue
        
    # See if it has the specific values
    has_42 = 0x2A in vals
    has_43 = 0x2B in vals
    has_44 = 0x2C in vals
    
    if has_43 or has_42:
        # Check if it decreases
        # we don't require strictly monotonically decreasing because of noise, but let's check the transitions
        # We can just print out the history for this byte index if it contains 42 or 43.
        # To avoid spam, only print if the values are generally in the [0, 100] range and it has 42 or 43.
        
        candidates.append((key, changes))
            
print(f"Found {len(candidates)} candidate byte indices that hit 42 or 43 and change:")
for key, changes in candidates:
    print(f"Length {key[0]}, Index {key[1]}:")
    for f, v in changes:
        print(f"  Frame {f}: {v}")
