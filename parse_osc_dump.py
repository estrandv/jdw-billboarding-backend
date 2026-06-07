#!/usr/bin/env python3
"""Read pcap from stdin, decode OSC packets, print human-readable."""
import struct
import sys
import os
from datetime import datetime, timezone

COMPACT = '--compact' in sys.argv
if COMPACT:
    sys.argv.remove('--compact')

def osc_pad(x):
    return (x + 3) & ~3

def read_osc_string(data, offset):
    end = data.index(b'\0', offset)
    s = data[offset:end].decode('utf-8', errors='replace')
    return s, osc_pad(end + 1)

def read_osc_data(data, offset, size):
    return data[offset:offset+size], osc_pad(offset + size)

def fmt_timetag(hi, lo):
    secs = hi + (lo / 2**32)
    try:
        dt = datetime.fromtimestamp(secs - 2208988800, tz=timezone.utc)
        return dt.strftime('%H:%M:%S.%f')[:-3]
    except:
        return f'{hi}.{lo:08x}'

def decode_osc(msg):
    out = []
    compact_parts = []
    addr, off = read_osc_string(msg, 0)
    out.append(f'  ADDR: {addr}')
    compact_parts.append(addr)

    if off >= len(msg):
        if COMPACT:
            return ['  '.join(compact_parts)]
        return out + ['  (no args)']
    if msg[off:off+1] != b',':
        if COMPACT:
            return ['  '.join(compact_parts) + '  (bad tags)']
        return out + [f'  (bad type tag at {off})']

    tags, off = read_osc_string(msg, off)
    out.append(f'  TAGS: {tags}')
    for t in tags[1:]:
        if t == 'i':
            val = struct.unpack_from('>i', msg, off)[0]
            out.append(f'    int {val}')
            compact_parts.append(f'int:{val}')
            off += 4
        elif t == 'f':
            val = struct.unpack_from('>f', msg, off)[0]
            out.append(f'    float {val}')
            compact_parts.append(f'float:{val}')
            off += 4
        elif t == 's':
            s, off = read_osc_string(msg, off)
            out.append(f'    str "{s}"')
            compact_parts.append(f'str:{s}')
        elif t == 'b':
            size = struct.unpack_from('>i', msg, off)[0]
            off += 4
            data, off = read_osc_data(msg, off, size)
            out.append(f'    blob {size} bytes')
            compact_parts.append(f'blob:{size}')
        elif t == 'm':
            midi_buf = msg[off:off+4]
            out.append(f'    midi {midi_buf.hex()}')
            compact_parts.append(f'midi:{midi_buf.hex()}')
            off += 4
        elif t == 't':
            hi = struct.unpack_from('>I', msg, off)[0]
            lo = struct.unpack_from('>I', msg, off + 4)[0]
            out.append(f'    timetag {fmt_timetag(hi, lo)}')
            compact_parts.append(f'timetag:{hi}.{lo:08x}')
            off += 8
        else:
            out.append(f'    (unknown type {t})')
            compact_parts.append(f'unknown:{t}')
    if COMPACT:
        return ['  '.join(compact_parts)]
    return out


def decode_bundle(data, depth=0):
    prefix = '  ' * depth
    if COMPACT:
        lines = []
    else:
        lines = [f'{prefix}=== BUNDLE ===']
    off = 16  # skip "#bundle\0" (8) + timetag (8)
    while off < len(data):
        size = struct.unpack_from('>i', data, off)[0]
        off += 4
        item = data[off:off+size]
        off += size
        if item.startswith(b'#bundle'):
            lines += decode_bundle(item, depth + 1)
        else:
            lines += decode_osc(item)
    return lines

def main():
    # Read pcap from stdin (tcpdump -w -)
    # Skip global header if present (24 bytes for pcap)
    data = sys.stdin.buffer.read()
    if not data:
        return
    # Check for pcap header magic
    if len(data) >= 24:
        magic = struct.unpack_from('<I', data, 0)[0]
        if magic in (0xa1b2c3d4, 0xd4c3b2a1, 0xa1b23c4d, 0x4d3cb2a1):
            # pcap global header present - skip per-packet headers
            off = 24
            pkt_n = 0
            while off < len(data):
                if off + 16 > len(data):
                    break
                ts_sec = struct.unpack_from('<I', data, off)[0]
                ts_usec = struct.unpack_from('<I', data, off + 4)[0]
                incl_len = struct.unpack_from('<I', data, off + 8)[0]
                orig_len = struct.unpack_from('<I', data, off + 12)[0]
                off += 16
                if off + incl_len > len(data):
                    break
                pkt = data[off:off+incl_len]
                off += incl_len
                dt = datetime.fromtimestamp(ts_sec, tz=timezone.utc)
                ts = dt.strftime('%H:%M:%S') + f'.{ts_usec:06d}'
                # Skip ethernet (14) + IP (20) + UDP (8) headers
                if len(pkt) < 42:
                    continue
                udp_payload = pkt[42:]
                pkt_n += 1
                if not COMPACT:
                    print(f'\n--- Packet {pkt_n} @ {ts} ({len(udp_payload)} bytes) ---')
                if udp_payload.startswith(b'#bundle'):
                    for line in decode_bundle(udp_payload):
                        print(line)
                else:
                    for line in decode_osc(udp_payload):
                        print(line)
            return
    # Raw OSC data
    pkt_n = 0
    off = 0
    while off < len(data):
        if data[off:off+1] == b'#':
            end = data.index(b'\0', off) if b'\0' in data[off:] else len(data)
            marker = data[off:end].decode()
            pkt_n += 1
            print(f'\n--- Packet {pkt_n} (raw) ---')
            if marker == '#bundle':
                for line in decode_bundle(data[off:]):
                    print(line)
                break
        pkt_n += 1
        print(f'\n--- Packet {pkt_n} (raw) ---')
        for line in decode_osc(data[off:]):
            print(line)
        break

if __name__ == '__main__':
    main()
