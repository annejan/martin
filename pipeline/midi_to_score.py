#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
# SPDX-License-Identifier: MIT
"""Extract a monophonic melody (+ bass) from a Standard MIDI File into martin's tracker note-grid
(16 slots/bar, `A4`/`C#5` = note · `.` = rest · `-` = tie), so a real tune is transcribed 1:1 instead
of from memory. Prints BPM, bar count, the key guess, and the per-bar lead/bass lines to paste into a
`<section>.lead p0:` block.

    pipeline/midi_to_score.py assets/beach/1352.mid [--lane lead|bass] [--minpitch 48] [--transpose 0]
"""
import argparse
import struct
import sys

NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]


def varlen(d, i):
    v = 0
    while True:
        b = d[i]; i += 1
        v = (v << 7) | (b & 0x7F)
        if not (b & 0x80):
            return v, i


def parse(path):
    with open(path, "rb") as f:
        d = f.read()
    assert d[:4] == b"MThd", "not a MIDI"
    fmt, ntrk, div = struct.unpack(">HHH", d[8:14])
    tempo = 500000  # default 120 bpm (us per quarter)
    notes = []  # (start_tick, pitch, channel)
    on = {}  # (chan,pitch) -> start_tick
    pos = 14
    for _ in range(ntrk):
        if d[pos:pos + 4] != b"MTrk":
            break
        ln = struct.unpack(">I", d[pos + 4:pos + 8])[0]
        body = d[pos + 8:pos + 8 + ln]
        i = 0; t = 0; status = 0
        while i < len(body):
            dt, i = varlen(body, i)
            t += dt
            b = body[i]
            if b & 0x80:
                status = b; i += 1
            ev = status & 0xF0; ch = status & 0x0F
            if status == 0xFF:  # meta
                meta = body[i]; i += 1
                mlen, i = varlen(body, i)
                if meta == 0x51:  # set tempo
                    tempo = int.from_bytes(body[i:i + 3], "big")
                i += mlen
            elif status in (0xF0, 0xF7):
                mlen, i = varlen(body, i)
                i += mlen
            elif ev == 0x90:  # note on
                pitch, vel = body[i], body[i + 1]; i += 2
                if vel > 0:
                    on[(ch, pitch)] = t
                else:
                    s = on.pop((ch, pitch), None)
                    if s is not None:
                        notes.append((s, t, pitch, ch))
            elif ev == 0x80:  # note off
                pitch = body[i]; i += 2
                s = on.pop((ch, pitch), None)
                if s is not None:
                    notes.append((s, t, pitch, ch))
            elif ev in (0xA0, 0xB0, 0xE0):
                i += 2
            elif ev in (0xC0, 0xD0):
                i += 1
            else:
                i += 1
        pos += 8 + ln
    bpm = round(60_000_000 / tempo)
    return div, bpm, notes


def grid(notes, div, lane, minp, maxp, transpose, channel=None):
    """Monophonic 16th grid. lane='lead' → highest note in [minp,maxp]; 'bass' → lowest.
    channel filters to one MIDI channel (the melody/bass channel) when given."""
    spb = div // 4  # ticks per 16th slot
    if spb == 0:
        spb = 1
    sel = [n for n in notes if minp <= n[2] <= maxp and (channel is None or n[3] == channel)]
    last_tick = max((n[1] for n in sel), default=0)
    nslots = (last_tick // spb) + 1
    nslots = ((nslots + 15) // 16) * 16  # round up to whole bars
    slots = [None] * nslots  # None=rest, ("note",pitch), ("tie",)
    # onset per slot: the chosen note that STARTS in this slot
    starts = {}
    for s, e, p, c in sel:
        sl = round(s / spb)
        if sl >= nslots:
            continue
        cur = starts.get(sl)
        better = cur is None or (p > cur[0] if lane == "lead" else p < cur[0])
        if better:
            starts[sl] = (p, e)
    held_until = -1
    held_slot = None
    for sl in range(nslots):
        if sl in starts:
            p, e = starts[sl]
            slots[sl] = ("note", p + transpose)
            held_until = round(e / spb)
            held_slot = sl
        elif held_slot is not None and sl < held_until:
            slots[sl] = ("tie",)
        else:
            slots[sl] = None
    return slots


def tok(s):
    if s is None:
        return "."
    if s[0] == "tie":
        return "-"
    p = s[1]
    return f"{NOTE_NAMES[p % 12]}{p // 12 - 1}"


def fmt_bars(slots):
    out = []
    for b in range(0, len(slots), 16):
        bar = slots[b:b + 16]
        groups = ["".ljust(0)]
        cells = [tok(x).rjust(3) for x in bar]
        line = "  ".join(" ".join(cells[g:g + 4]) for g in range(0, 16, 4))
        out.append(line)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("midi")
    ap.add_argument("--lane", choices=["lead", "bass"], default="lead")
    ap.add_argument("--minpitch", type=int, default=55)
    ap.add_argument("--maxpitch", type=int, default=96)
    ap.add_argument("--transpose", type=int, default=0)
    ap.add_argument("--channel", type=int, default=None, help="filter to one MIDI channel")
    ap.add_argument("--bars", type=int, default=0, help="print only the first N bars (0=all)")
    a = ap.parse_args()
    div, bpm, notes = parse(a.midi)
    pitches = [n[2] for n in notes]
    # crude key guess: most common pitch class
    from collections import Counter
    pc = Counter(p % 12 for p in pitches)
    key = NOTE_NAMES[pc.most_common(1)[0][0]] if pc else "?"
    print(f"# {a.midi}: bpm {bpm}, div {div}, {len(notes)} notes, pitch {min(pitches)}-{max(pitches)}, key~{key}", file=sys.stderr)
    slots = grid(notes, div, a.lane, a.minpitch, a.maxpitch, a.transpose, a.channel)
    bars = fmt_bars(slots)
    if a.bars:
        bars = bars[:a.bars]
    print(f"# {len(bars)} bars, lane={a.lane}", file=sys.stderr)
    for i, ln in enumerate(bars):
        print(f"  {ln}    # bar {i+1}")


if __name__ == "__main__":
    main()
