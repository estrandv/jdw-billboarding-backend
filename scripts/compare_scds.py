#!/usr/bin/env python3
"""Compare NRT SCD files (old/correct vs new/rust). Two-pass diff:

  PASS 1 (automatic): structural identity — synthdef set, entry counts,
      command breakdown, duration. Any difference is substantial.
  PASS 2 (with --full): detailed times and /s_new args.

Usage:
  python3 compare_scds.py                          # batch all 23 pairs
  python3 compare_scds.py old.scd new.scd          # single pair
  python3 compare_scds.py --full old.scd new.scd   # single pair, show detail
  python3 compare_scds.py --full                   # batch all 23 pairs, full
"""

import re
import sys
import os
import hashlib
from collections import Counter

def parse_scd(path):
    """Parse SCD file, return dict with structural info."""
    with open(path) as f:
        content = f.read()

    sha = hashlib.sha256(content.encode()).hexdigest()[:12]

    duration = None
    m = re.search(r'duration:\s*([\d.]+)', content)
    if m:
        duration = m.group(1)

    synthdefs = set(re.findall(r'SynthDef\.new\("([^"]+)"', content))

    entries = []
    in_score = False
    for line in content.split('\n'):
        stripped = line.strip()
        if stripped.startswith('a = Score(['):
            in_score = True
            continue
        if not in_score:
            continue
        if stripped in (']);', ']);'):
            break
        m = re.match(r"\s*\[\s*([\d.]+)\s*,\s*\[\s*[\"']([^\"']+)[\"']", line)
        if m:
            entries.append((float(m.group(1)), m.group(2), line))

    cmd_counts = Counter(e for _, e, _ in entries)
    times = [e[0] for e in entries] if entries else []

    s_new_args = []
    for t, cmd, line in entries:
        if cmd == '/s_new':
            sm = re.search(r'"/s_new"\s*,\s*"([^"]+)"', line)
            if not sm:
                sm = re.search(r"'/s_new'\s*,\s*'([^']+)'", line)
            fm = re.search(r'"freq"\s*,\s*([\d.]+)', line)
            bm = re.search(r'"buf"\s*,\s*(\d+)', line)
            am = re.search(r'"amp"\s*,\s*([\d.]+)', line)
            s_new_args.append((
                sm.group(1) if sm else '?',
                float(fm.group(1)) if fm else None,
                int(bm.group(1)) if bm else None,
                float(am.group(1)) if am else None,
            ))

    return {
        'sha': sha, 'entries': entries, 'times': times,
        'cmd_counts': dict(cmd_counts), 'duration': duration,
        'synthdefs': synthdefs, 's_new_args': s_new_args, 'path': path,
    }


def pass1(old, new):
    """Structural identity check. Any difference is a real mismatch."""
    issues = []
    o_s = old['synthdefs']; n_s = new['synthdefs']
    if o_s != n_s:
        issues.append(f"synthdefs differ: old={sorted(o_s)} new={sorted(n_s)}")
    no, nn = len(old['entries']), len(new['entries'])
    if no != nn:
        issues.append(f"entries: old={no} new={nn}")
    do = float(old['duration'] or 0); dn = float(new['duration'] or 0)
    if abs(do - dn) > 0.5:
        issues.append(f"duration: old={do} new={dn}")
    all_cmds = set(old['cmd_counts']) | set(new['cmd_counts'])
    for c in sorted(all_cmds):
        oc = old['cmd_counts'].get(c, 0); nc = new['cmd_counts'].get(c, 0)
        if oc != nc:
            issues.append(f"cmd /{c}: old={oc} new={nc}")
    return issues


def compare(old_path, new_path, full=False):
    try:
        old = parse_scd(old_path)
        new = parse_scd(new_path)
    except FileNotFoundError as e:
        print(f"  ERROR: {e}")
        return

    print(f"  old sha={old['sha']}  new sha={new['sha']}")

    issues = pass1(old, new)
    if issues:
        print(f"  PASS 1 FAILED:")
        for iss in issues:
            print(f"    - {iss}")
        if not full:
            return

    if not issues or full:
        print(f"  PASS 2 (detail):")
        if old['times'] and new['times']:
            print(f"    times:  old={[f'{t:.2f}' for t in old['times'][:5]]} ... {[f'{t:.2f}' for t in old['times'][-3:]]}")
            print(f"            new={[f'{t:.2f}' for t in new['times'][:5]]} ... {[f'{t:.2f}' for t in new['times'][-3:]]}")
        if old['s_new_args'] and new['s_new_args']:
            print(f"    /s_new (first 3):")
            for i in range(min(3, len(old['s_new_args']), len(new['s_new_args']))):
                so, fo, bo, ao = old['s_new_args'][i]
                sn, fn, bn, an = new['s_new_args'][i]
                print(f"      [{i}] old: {so} freq={fo} buf={bo}")
                print(f"      [{i}] new: {sn} freq={fn} buf={bn}")

    print()


if __name__ == '__main__':
    full = '--full' in sys.argv
    args = [a for a in sys.argv[1:] if a != '--full']

    if len(args) == 2:
        compare(args[0], args[1], full=full)
    else:
        old_dir = os.path.expanduser('~/tmp/nrt_bug_export/old_method')
        new_dir = os.path.expanduser('~/jdw_output')
        PAIRS = [
            ("track_aPad_apad_0.wav.scd", "track_aPad_8_0.wav.scd"),
            ("track_blip_chorus_0.wav.scd", "track_blip_2_0.wav.scd"),
            ("track_eBass_cbass_1.wav.scd", "track_cbass.wav.scd"),
            ("track_eBass_dbass_0.wav.scd", "track_eBass_4_0.wav.scd"),
            ("track_EMU_SP12_drumx_0.wav.scd", "track_EMU_SP12_13_0.wav.scd"),
            ("track_EMU_SP12_drumi_0.wav.scd", "track_EMU_SP12_3_0.wav.scd"),
            ("track_EMU_SP12_drumi_1.wav.scd", "track_EMU_SP12_13_1.wav.scd"),
            ("track_EMU_SP12_drumi_2.wav.scd", "track_EMU_SP12_13_2.wav.scd"),
            ("track_EMU_SP12_drumi_3.wav.scd", "track_EMU_SP12_13_3.wav.scd"),
            ("track_EMU_SP12_cdrum_1.wav.scd", "track_cdrum.wav.scd"),
            ("track_experimental_brah_0.wav.scd", "track_experimental_1_0.wav.scd"),
            ("track_FMRhodes_rat_0.wav.scd", "track_FMRhodes_7_0.wav.scd"),
            ("track_FMRhodes_rhodesii_0.wav.scd", "track_FMRhodes_5_0.wav.scd"),
            ("track_gritBass_gritBass_0.wav.scd", "track_gritBass_0_0.wav.scd"),
            ("track_gritBass_gritBass_1.wav.scd", "track_gritBass_0_1.wav.scd"),
            ("track_karp_blip_0.wav.scd", "track_karp_10_0.wav.scd"),
            ("track_karp_blipii_0.wav.scd", "track_karp_9_0.wav.scd"),
            ("track_moogBass_moog_0.wav.scd", "track_moogBass_6_0.wav.scd"),
            ("track_pluck_pluck_3.wav.scd", "track_pluck_11_3.wav.scd"),
            ("track_pluck_vocbridge_0.wav.scd", "track_vocbridge.wav.scd"),
            ("track_pluck_vocchorus_2.wav.scd", "track_vocchorus.wav.scd"),
            ("track_pluck_vocverse_1.wav.scd", "track_vocverse.wav.scd"),
            ("track_Roland808_drum_0.wav.scd", "track_Roland808_12_0.wav.scd"),
        ]
        for old_file, new_file in PAIRS:
            print(f"{'='*60}")
            print(f"PAIR: {old_file}  <->  {new_file}")
            print(f"{'='*60}")
            compare(os.path.join(old_dir, old_file), os.path.join(new_dir, new_file), full=full)
