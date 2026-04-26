import sys

out_frames = {}
with open('/tmp/pulsar_out.txt', 'r') as f:
    for line in f:
        parts = line.strip().split()
        if len(parts) >= 2:
            out_frames[int(parts[0])] = parts[1]

in_frames = {}
with open('/tmp/pulsar_in.txt', 'r') as f:
    for line in f:
        parts = line.strip().split()
        if len(parts) >= 2:
            in_frames[int(parts[0])] = parts[1]

# Sort by frame number
all_frames = sorted(list(out_frames.items()) + list(in_frames.items()), key=lambda x: x[0])

# Let's print the sequence around the 0801 responses
for i, (fn, data) in enumerate(all_frames):
    if data.startswith("0801") and fn in in_frames:
        # Print 5 frames before this
        print(f"\n--- Sequence leading to IN 0801 at frame {fn} (data: {data}) ---")
        start = max(0, i - 10)
        for j in range(start, i + 1):
            f_num, d = all_frames[j]
            dir_str = "OUT" if f_num in out_frames else "IN "
            print(f"Frame {f_num:5d} {dir_str}: {d}")
