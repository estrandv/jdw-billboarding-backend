#!/usr/bin/env bash
# capture_compare.sh — single-run, non-interactive Python vs Rust OSC comparison
# Usage: ./capture_compare.sh [song.bbd]
# Default: arena.bbd from jdw-pycompose
#
# Requires: sudo tcpdump, $MYPYTHON, cargo
# Runs: setup → update → play in one shot with a single sudo tcpdump.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PYCOMPOSE="/home/estrandv/programming/jdw-pycompose"
MYPYTHON="$HOME/mypython/bin/python"
SONG="${1:-$PYCOMPOSE/songs/arena.bbd}"
SONG_NAME="$(basename "$SONG")"
TMPDIR="/tmp/osc_compare_$$"
PCAP="$TMPDIR/all.pcap"
PCAP_PORT=13339

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
info() { echo -e "${GREEN}[*]${NC} $*"; }
warn() { echo -e "${YELLOW}[!]${NC} $*"; }
header(){ echo -e "\n${YELLOW}=== $* ===${NC}"; }

mkdir -p "$TMPDIR"
cleanup() { rm -rf "$TMPDIR"; }
trap cleanup EXIT

header "Starting single-run comparison for: $SONG_NAME"

# ---- Start tcpdump (single sudo, captures everything) ----
info "Starting tcpdump (sudo) on port $PCAP_PORT..."
sudo tcpdump -i lo -U -w "$PCAP" udp port "$PCAP_PORT" &
TCPDUMP_PID=$!
sleep 0.5  # let tcpdump initialize

# ---- Tiny sentinel helper (valid OSC /phase_marker message) ----
send_marker() {
    $MYPYTHON -c "
import socket, struct, sys
tag = sys.argv[1] if len(sys.argv)>1 else '?'
addr = b'/phase_marker\x00\x00'
_    = b',s\x00\x00'
msg  = tag.encode() + b'\x00' * (4 - len(tag.encode()) % 4)
sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.sendto(addr + _ + msg, ('127.0.0.1', $PCAP_PORT))
sock.close()
" "$1" 2>/dev/null
}

# ---- Phase 1: SETUP ----
info "Phase 1: SETUP"
send_marker setup_start
(cd "$PYCOMPOSE" && $MYPYTHON run.py --setup "$SONG")
sleep 1

# ---- Phase 2: UPDATE ----
info "Phase 2: UPDATE"
send_marker update_start
(cd "$PYCOMPOSE" && $MYPYTHON run.py --update "$SONG")
sleep 1

# ---- Phase 3: PLAY ----
info "Phase 3: PLAY"
send_marker play_start
(cd "$PYCOMPOSE" && $MYPYTHON run.py "$SONG")
sleep 1

# ---- Stop capture ----
send_marker done
sleep 0.5
sudo kill "$TCPDUMP_PID" 2>/dev/null || true
wait "$TCPDUMP_PID" 2>/dev/null || true
info "Capture complete ($(stat -c%s "$PCAP" 2>/dev/null || echo 0) bytes)"

# ---- Parse pcap and split by phase markers ----
header "Parsing capture"
$MYPYTHON "$SCRIPT_DIR/parse_osc_dump.py" --compact < "$PCAP" > "$TMPDIR/all.txt" 2>/dev/null
ALL_MSGS=$(wc -l < "$TMPDIR/all.txt" 2>/dev/null || echo 0)
info "Total messages captured: $ALL_MSGS"

# ---- Split by phase markers ----
$MYPYTHON -c "
import sys
lines = open('$TMPDIR/all.txt').readlines()

# Find marker indices
# Marker format: /phase_marker  str:setup_start  (or similar)
marker_positions = {}
for i, line in enumerate(lines):
    if '/phase_marker' in line:
        # extract the phase name from the string arg
        if 'str:setup_start' in line:
            marker_positions['setup_start'] = i
        elif 'str:update_start' in line:
            marker_positions['update_start'] = i
        elif 'str:play_start' in line:
            marker_positions['play_start'] = i
        elif 'str:done' in line:
            marker_positions['done'] = i

# Write per-phase files
phases = [
    ('setup',  'setup_start',  'update_start'),
    ('update', 'update_start', 'play_start'),
    ('play',   'play_start',   'done'),
]
for name, start_marker, end_marker in phases:
    if start_marker in marker_positions and end_marker in marker_positions:
        s = marker_positions[start_marker] + 1  # skip marker itself
        e = marker_positions[end_marker]
        with open('$TMPDIR/python_' + name + '.txt', 'w') as f:
            f.writelines(lines[s:e])
        print(f'Python {name}: {e-s} messages')
" 2>/dev/null

# ---- Dump Rust equivalents ----
header "Dumping Rust messages"
(cd "$SCRIPT_DIR" && cargo build --example dump_osc 2>/dev/null) || true

info "Rust setup dump..."
(cd "$SCRIPT_DIR" && cargo run --example dump_osc -- --phase setup "$SONG" 2>/dev/null) > "$TMPDIR/rust_setup.txt"
info "Rust commands dump..."
(cd "$SCRIPT_DIR" && cargo run --example dump_osc -- --phase commands "$SONG" 2>/dev/null) > "$TMPDIR/rust_commands.txt"
info "Rust play dump..."
(cd "$SCRIPT_DIR" && cargo run --example dump_osc -- --phase play "$SONG" 2>/dev/null) > "$TMPDIR/rust_play.txt"

# ---- Quick comparison ----
header "Summary"

echo ""
printf "%-22s | %-10s | %-6s\n" "File" "Phase" "Msgs"
printf "%-22s-+-%-10s-+-%-6s\n" "----------------------" "----------" "------"
for name in setup update play; do
    if [[ -f "$TMPDIR/python_${name}.txt" ]]; then
        printf "%-22s | %-10s | %-6s\n" "python_${name}.txt" "$name" "$(wc -l < "$TMPDIR/python_${name}.txt")"
    fi
done
printf "%-22s-+-%-10s-+-%-6s\n" "----------------------" "----------" "------"
printf "%-22s | %-10s | %-6s\n" "rust_setup.txt" "setup" "$(grep -c '\[/' "$TMPDIR/rust_setup.txt" 2>/dev/null || echo 0)"
printf "%-22s | %-10s | %-6s\n" "rust_commands.txt" "commands" "$(grep -c '\[/' "$TMPDIR/rust_commands.txt" 2>/dev/null || echo 0)"
printf "%-22s | %-10s | %-6s\n" "rust_play.txt" "play" "$(grep -c '\[/' "$TMPDIR/rust_play.txt" 2>/dev/null || echo 0)"

# ---- Message type breakdowns ----
echo ""
info "Message type breakdown per phase"

count_by_addr() {
    awk -F'  ' '{print $1}' "$1" 2>/dev/null | sort | uniq -c | sort -rn | head -15
}

for name in setup update play; do
    f="$TMPDIR/python_${name}.txt"
    if [[ -f "$f" ]] && [[ -s "$f" ]]; then
        echo ""
        echo "=== Python $name messages ==="
        count_by_addr "$f"
    fi
done

echo ""
echo "=== Rust PLAY messages ==="
grep '^  \[' "$TMPDIR/rust_play.txt" | grep -oP '/\S+' | sort | uniq -c | sort -rn

echo ""
echo "=== Rust COMMANDS messages ==="
grep '^  \[' "$TMPDIR/rust_commands.txt" | grep -oP '/\S+' | sort | uniq -c | sort -rn

echo ""
echo "=== Rust SETUP messages ==="
grep '^  \[' "$TMPDIR/rust_setup.txt" | grep -oP '/\S+' | sort | uniq -c | sort -rn

# ---- File paths ----
trap - EXIT
echo ""
info "Files preserved in: $TMPDIR"
echo "  python_setup.txt   python_update.txt   python_play.txt"
echo "  rust_setup.txt     rust_commands.txt   rust_play.txt"
echo ""
info "To diff e.g. update phase:"
echo "  awk '{print \$1}' $TMPDIR/python_update.txt | sort | uniq -c"
echo "  # vs:"
echo "  grep '^  \[' $TMPDIR/rust_commands.txt | grep -oP '/\\S+' | sort | uniq -c"
