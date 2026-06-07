#!/usr/bin/env python3
"""Convert Rust dump_osc output to compact OSC format for diffing."""
import re
import sys

for line in sys.stdin:
    line = line.rstrip()
    # Match:   [N] /address arg1 arg2 ...
    # Args can be: "string"  or  number  or  float
    m = re.match(r'^\s*\[\d+\]\s+(/\S+)\s+(.*)', line)
    if not m:
        continue
    addr = m.group(1)
    rest = m.group(2)
    # Skip (t=X) timing annotation
    rest = re.sub(r'^\(t=[^)]+\)\s*', '', rest)
    # Parse the rest into typed args
    parts = [addr]
    tokens = re.findall(r'"([^"]*)"|(\d+\.\d+)|(\d+)|(\S+)', rest)
    for t in tokens:
        s, f, i, u = t
        if s:
            parts.append(f'str:{s}')
        elif f:
            parts.append(f'float:{f}')
        elif i:
            parts.append(f'int:{i}')
        elif u:
            parts.append(f'str:{u}')
    sys.stdout.write('  '.join(parts) + '\n')
