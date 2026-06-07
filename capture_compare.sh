#!/usr/bin/env bash
# capture_compare.sh — capture Python OSC traffic and compare with Rust dumps
# Usage: ./capture_compare.sh [song.bbd]
# Default: arena.bbd from jdw-pycompose

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PYCOMPOSE="/home/estrandv/programming/jdw-pycompose"
SONG="${1:-$PYCOMPOSE/songs/arena.bbd}"
SONG_NAME="$(basename "$SONG")"
TMPDIR="/tmp/osc_compare_$$"
PCAP_PORT=13339  # Python osc-router port

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

mkdir -p "$TMPDIR"

cleanup() {
    rm -rf "$TMPDIR"
    echo ""
    echo "Cleaned up $TMPDIR"
}
trap cleanup EXIT

info()  { echo -e "${GREEN}[*]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!]${NC} $*"; }
err()   { echo -e "${RED}[X]${NC} $*"; }
header(){ echo -e "\n${YELLOW}=== $* ===${NC}"; }

# ---- Phase 1: Setup (synthdefs + configure) ----
header "Phase 1: Python SETUP"
info "Song: $SONG"
echo ""
echo "RUN THIS IN A SEPARATE TERMINAL:"
echo "  sudo tcpdump -i lo -U -w $TMPDIR/setup.pcap udp port $PCAP_PORT"
echo ""
read -p "Press ENTER when tcpdump is running, then run this command in another terminal:" _
echo ""
echo "  cd $PYCOMPOSE && python3 run.py --setup \"$SONG\""
echo ""
read -p "Press ENTER when the Python setup command has completed, then Ctrl+C tcpdump:" _

if [[ -f "$TMPDIR/setup.pcap" ]] && [[ $(stat -c%s "$TMPDIR/setup.pcap" 2>/dev/null) -gt 40 ]]; then
    info "Setup pcap captured ($(stat -c%s "$TMPDIR/setup.pcap") bytes)"
else
    warn "Setup pcap might be empty or missing!"
fi

# ---- Phase 2: Update/Configure (commands + effects + drones) ----
header "Phase 2: Python UPDATE (configure)"
info "Song: $SONG"
echo ""
echo "RUN THIS IN A SEPARATE TERMINAL:"
echo "  sudo tcpdump -i lo -U -w $TMPDIR/update.pcap udp port $PCAP_PORT"
echo ""
read -p "Press ENTER when tcpdump is running, then run in another terminal:" _
echo ""
echo "  cd $PYCOMPOSE && python3 run.py --update \"$SONG\""
echo ""
read -p "Press ENTER when the Python update command has completed, then Ctrl+C tcpdump:" _

if [[ -f "$TMPDIR/update.pcap" ]] && [[ $(stat -c%s "$TMPDIR/update.pcap" 2>/dev/null) -gt 40 ]]; then
    info "Update pcap captured ($(stat -c%s "$TMPDIR/update.pcap") bytes)"
else
    warn "Update pcap might be empty or missing!"
fi

# ---- Phase 3: Play (queue update) ----
header "Phase 3: Python PLAY"
echo ""
echo "RUN THIS IN A SEPARATE TERMINAL:"
echo "  sudo tcpdump -i lo -U -w $TMPDIR/play.pcap udp port $PCAP_PORT"
echo ""
read -p "Press ENTER when tcpdump is running, then run in another terminal:" _
echo ""
echo "  cd $PYCOMPOSE && python3 run.py \"$SONG\""
echo ""
read -p "Press ENTER when the Python song has completed (or Ctrl+C if stuck), then Ctrl+C tcpdump:" _

if [[ -f "$TMPDIR/play.pcap" ]] && [[ $(stat -c%s "$TMPDIR/play.pcap" 2>/dev/null) -gt 40 ]]; then
    info "Play pcap captured ($(stat -c%s "$TMPDIR/play.pcap") bytes)"
else
    warn "Play pcap might be empty or missing!"
fi

# ---- Parse captures ----
header "Parsing captures"

info "Parsing setup pcap..."
python3 "$SCRIPT_DIR/parse_osc_dump.py" --compact < "$TMPDIR/setup.pcap" > "$TMPDIR/python_setup.txt" 2>/dev/null || warn "Setup parse failed (maybe empty)"
echo "  Python setup: $(wc -l < "$TMPDIR/python_setup.txt" 2>/dev/null || echo 0) messages"

info "Parsing update pcap..."
python3 "$SCRIPT_DIR/parse_osc_dump.py" --compact < "$TMPDIR/update.pcap" > "$TMPDIR/python_update.txt" 2>/dev/null || warn "Update parse failed (maybe empty)"
echo "  Python update: $(wc -l < "$TMPDIR/python_update.txt" 2>/dev/null || echo 0) messages"

info "Parsing play pcap..."
python3 "$SCRIPT_DIR/parse_osc_dump.py" --compact < "$TMPDIR/play.pcap" > "$TMPDIR/python_play.txt" 2>/dev/null
echo "  Python play: $(wc -l < "$TMPDIR/python_play.txt") messages"

# ---- Dump Rust messages ----
header "Dumping Rust messages"

info "Building Rust dump_osc..."
(cd "$SCRIPT_DIR" && cargo build --example dump_osc 2>/dev/null) || true

info "Rust setup dump..."
(cd "$SCRIPT_DIR" && cargo run --example dump_osc -- --phase setup "$SONG" 2>/dev/null) > "$TMPDIR/rust_setup.txt"
echo "  Rust setup: $(grep -c '\[/' "$TMPDIR/rust_setup.txt" 2>/dev/null || echo 0) messages"

info "Rust commands dump..."
(cd "$SCRIPT_DIR" && cargo run --example dump_osc -- --phase commands "$SONG" 2>/dev/null) > "$TMPDIR/rust_commands.txt"
echo "  Rust commands: $(grep -c '\[/' "$TMPDIR/rust_commands.txt" 2>/dev/null || echo 0) messages"

info "Rust play dump..."
(cd "$SCRIPT_DIR" && cargo run --example dump_osc -- --phase play "$SONG" 2>/dev/null) > "$TMPDIR/rust_play.txt"
echo "  Rust play: $(grep -c '\[/' "$TMPDIR/rust_play.txt" 2>/dev/null || echo 0) messages"

# ---- Quick comparison ----
header "Quick Comparison"

echo ""
echo "File           | Messages"
echo "---------------|----------"
for f in python_setup python_update python_play rust_setup rust_commands rust_play; do
    if [[ -f "$TMPDIR/${f}.txt" ]]; then
        printf "%-15s| %s\n" "$f" "$(wc -l < "$TMPDIR/${f}.txt")"
    fi
done

echo ""
info "Message type breakdowns..."

# Address counts
count_by_addr() {
    awk '{print $1}' "$1" | sort | uniq -c | sort -rn | head -15
}

echo ""
echo "=== Python SETUP message types ==="
count_by_addr "$TMPDIR/python_setup.txt" 2>/dev/null

echo ""
echo "=== Python UPDATE message types ==="
count_by_addr "$TMPDIR/python_update.txt" 2>/dev/null

echo ""
echo "=== Python PLAY message types ==="
count_by_addr "$TMPDIR/python_play.txt"

echo ""
echo "=== Rust COMMANDS message types ==="
grep '^  \[' "$TMPDIR/rust_commands.txt" | grep -oP '/\S+' | sort | uniq -c | sort -rn

echo ""
echo "=== Rust PLAY message types ==="
grep '^  \[' "$TMPDIR/rust_play.txt" | grep -oP '/\S+' | sort | uniq -c | sort -rn

echo ""
info "Full dump files saved in: $TMPDIR"
echo "  python_setup.txt   - Python synthdef loads + configure"
echo "  python_update.txt  - Python configure only (effects, drones, commands)"
echo "  python_play.txt    - Python queue update (notes)"
echo "  rust_setup.txt     - Rust synthdef setup"
echo "  rust_commands.txt  - Rust billboard commands"
echo "  rust_play.txt      - Rust queue update (notes)"
echo ""
info "Key comparisons to make:"
echo "  # Compare Python UPDATE (effects + drones) vs Rust COMMANDS:"
echo "  diff <(awk '{print \$1}' $TMPDIR/python_update.txt | sort) \\"
echo "       <(grep '^  \[' $TMPDIR/rust_commands.txt | grep -oP '/\\S+' | sort)"
echo ""
info "To see all Rust dumps:"
echo "  cd $SCRIPT_DIR"
echo "  cargo run --example dump_osc -- --phase all \"$SONG\""

# Don't trap with cleanup since we want files kept
trap - EXIT
echo ""
info "Files preserved in: $TMPDIR"
