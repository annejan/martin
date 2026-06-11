#!/usr/bin/env python3
"""Export assets/score.txt (martin's tracker-DSL arrangement) to a standard MIDI file so the
"Op de Camping" arrangement can be shared with / opened by composers & arrangers in any DAW or
notation tool. Reflects the engine's timeline: sections laid end to end, each melodic phrase looping
CONTINUOUSLY across its section (NoteLane::bar), per-section chord overrides.

Tracks: 1=Lead (horn), 2=Sax (arp, 8va below), 3=Bass, 4=Chords (triad pad).
Usage:  python3 pipeline/score_to_midi.py [assets/score.txt] [out.mid]

This is the project's own heavily-derived arrangement of "Op de Camping" (Ome Henk, 1995) / "In the
Navy" (Village People, 1979) — see REUSE.toml for source attribution. No lyrics are encoded.
"""
import sys, struct, re

SEMI = {'C': 0, 'D': 2, 'E': 4, 'F': 5, 'G': 7, 'A': 9, 'B': 11}
PPQ = 480
SIXTEENTH = PPQ // 4


def note_midi(tok):
    """'A#4' / 'Db3' / '.' -> MIDI number or None."""
    if tok in ('.', '-', '_'):
        return None
    m = re.match(r'^([A-Ga-g])([#b]?)(-?\d+)$', tok)
    if not m:
        return None
    s = SEMI[m.group(1).upper()] + (1 if m.group(2) == '#' else -1 if m.group(2) == 'b' else 0)
    return (int(m.group(3)) + 1) * 12 + s


def chord_notes(tok):
    """'Gm' / 'A#' / 'D' -> [root, third, fifth] MIDI numbers around octave 3."""
    minor = tok.endswith('m') and len(tok) > 1
    name = tok[:-1] if minor else tok
    root = note_midi(name + '3')
    if root is None:
        return []
    return [root, root + (3 if minor else 4), root + 7]


def parse(path):
    bpm, gchords, sections, order = 120.0, [], {}, []
    lead, arp, bass, schords = {}, {}, {}, {}
    for raw in open(path):
        line = raw.split('#')[0].strip() if not raw.lstrip().startswith('#') else ''
        # keep sharps: only strip a '#' that starts a comment (handled crudely: drop after ' #')
        line = re.split(r'\s#', raw)[0].strip()
        if raw.lstrip().startswith('#') or not line:
            continue
        tok = line.split()
        if tok[0] == 'bpm':
            bpm = float(tok[1])
        elif tok[0] == 'chords' and '.' not in tok[0]:
            gchords = tok[1:]
        elif tok[0] == 'section':
            order.append(tok[1])
            sections[tok[1]] = int(tok[2])
        elif '.' in tok[0]:
            head, _, rest = line.partition(':')
            sec, _, inst = head.strip().partition('.')
            inst = inst.split()[0]
            vals = rest.split()
            if inst == 'chords':
                schords[sec] = vals
            elif inst in ('lead', 'arp', 'bass') and head.strip().endswith('p0'):
                bars = [[note_midi(v) for v in vals[i:i + 16]] for i in range(0, len(vals), 16)]
                {'lead': lead, 'arp': arp, 'bass': bass}[inst][sec] = bars
    return bpm, gchords, order, sections, lead, arp, bass, schords


def vlq(n):
    out = bytearray([n & 0x7F]); n >>= 7
    while n:
        out.insert(0, (n & 0x7F) | 0x80); n >>= 7
    return bytes(out)


def track(events):
    """events: list of (abs_tick, status, d1, d2) -> MTrk bytes."""
    events.sort(key=lambda e: e[0])
    body, last = bytearray(), 0
    for tick, st, d1, d2 in events:
        body += vlq(tick - last) + bytes([st, d1, d2]); last = tick
    body += vlq(0) + b'\xff\x2f\x00'
    return b'MTrk' + struct.pack('>I', len(body)) + bytes(body)


def lane_events(phrase_by_sec, order, sections, ch, gate_until_next, vel=90):
    ev, bar0 = [], 0
    for sec in order:
        nb = sections[sec]
        bars = phrase_by_sec.get(sec)
        for b in range(nb):
            if bars:
                row = bars[b % len(bars)]
                for s, n in enumerate(row):
                    if n is None:
                        continue
                    on = (bar0 + b) * 16 * SIXTEENTH + s * SIXTEENTH
                    # duration: until the next non-rest slot in this bar (legato) or one 16th
                    dur = SIXTEENTH
                    if gate_until_next:
                        nxt = next((k for k in range(s + 1, 16) if row[k] is not None), 16)
                        dur = (nxt - s) * SIXTEENTH
                    ev.append((on, 0x90 | ch, n, vel))
                    ev.append((on + dur - 2, 0x80 | ch, n, 0))
        bar0 += nb
    return ev


def chord_events(gchords, schords, order, sections, ch=3):
    ev, bar0 = [], 0
    for sec in order:
        nb = sections[sec]
        prog = schords.get(sec, gchords)
        for b in range(nb):
            toks = prog[(b) % len(prog)] if sec in schords else prog[(bar0 + b) % len(prog)]
            for n in chord_notes(toks):
                on = (bar0 + b) * 16 * SIXTEENTH
                ev.append((on, 0x90 | ch, n, 64))
                ev.append((on + 16 * SIXTEENTH - 4, 0x80 | ch, n, 0))
        bar0 += nb
    return ev


def main():
    src = sys.argv[1] if len(sys.argv) > 1 else 'assets/score.txt'
    out = sys.argv[2] if len(sys.argv) > 2 else 'op-de-camping.mid'
    bpm, gchords, order, sections, lead, arp, bass, schords = parse(src)
    tempo = int(60_000_000 / bpm)
    meta = b'MTrk' + b''  # tempo/name track
    tbody = vlq(0) + b'\xff\x51\x03' + struct.pack('>I', tempo)[1:]
    tbody += vlq(0) + b'\xff\x03' + vlq(len(b'Op de Camping')) + b'Op de Camping'
    tbody += vlq(0) + b'\xff\x2f\x00'
    meta = b'MTrk' + struct.pack('>I', len(tbody)) + bytes(tbody)

    tracks = [
        meta,
        track(lane_events(lead, order, sections, 0, True, 96)),
        track(lane_events(arp, order, sections, 1, True, 78)),
        track(lane_events(bass, order, sections, 2, False, 88)),
        track(chord_events(gchords, schords, order, sections, 3)),
    ]
    header = b'MThd' + struct.pack('>IHHH', 6, 1, len(tracks), PPQ)
    with open(out, 'wb') as f:
        f.write(header + b''.join(tracks))
    total = sum(sections[s] for s in order)
    print(f'wrote {out}: {len(tracks)-1} music tracks, {total} bars @ {bpm:.0f} BPM '
          f'(lead/sax/bass/chords)')


if __name__ == '__main__':
    main()
