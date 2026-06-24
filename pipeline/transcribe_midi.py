#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
# SPDX-License-Identifier: MIT
"""Transcribe a public MIDI into martin's tracker DSL (a SECTIONED score.txt) — the midi→score inverse
of pipeline/score_to_midi.py. Quantises to martin's 16-slots-per-bar grid (NOT a from-memory guess —
see the camping/baby scores). Monophonic per lane: lead = highest voice of a melodic channel, bass =
lowest of the bass channel. Drums map GM percussion → kick/snare/hat. Stab = a chord-channel trigger.

    pipeline/transcribe_midi.py IN.mid OUT.txt --bpm 137 --lead 4 --bass 1 --stab 0 \
        --drums 9 --bars-per-section 8 --max-bars 0    (0 = whole song)

Emits per-section lead/bass/stab/kick/snare/hat phrases + per-bar chords from the bass root. A starting
point: hand-tune the arrangement/sections afterwards (it transcribes notes, not musical structure)."""
import argparse, collections, sys
import mido

NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]


def note_name(m):
    return f"{NAMES[m % 12]}{m // 12 - 1}"  # MIDI 69 → A4


def chord_of(m):  # bass root → chord token (letter only; octave dropped), minor by default (rave)
    return NAMES[m % 12] + "m"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("inp"); ap.add_argument("out")
    ap.add_argument("--bpm", type=float, default=0)
    ap.add_argument("--lead", type=int, default=4)
    ap.add_argument("--arp", type=int, default=-1)  # optional 2nd melodic lane (e.g. a riff)
    ap.add_argument("--bass", type=int, default=1)
    ap.add_argument("--stab", type=int, default=0)
    ap.add_argument("--drums", type=int, default=9)
    ap.add_argument("--bars-per-section", type=int, default=8)
    ap.add_argument("--max-bars", type=int, default=0)
    a = ap.parse_args()

    mid = mido.MidiFile(a.inp)
    ppq = mid.ticks_per_beat
    slot = ppq // 4          # 16th-note grid
    bar_slots = 16
    bpm = a.bpm
    # absolute-tick note events per channel: (start_slot, end_slot, note)
    ch_notes = collections.defaultdict(list)
    drum_hits = collections.defaultdict(set)  # slot -> {kick,snare,hat}
    open_notes = collections.defaultdict(dict)  # (ch,note) -> start_tick
    t = 0
    for msg in mido.merge_tracks(mid.tracks):
        t += msg.time
        if msg.type == "set_tempo" and bpm_unset(bpm):
            bpm = round(mido.tempo2bpm(msg.tempo), 3)
        if msg.type == "note_on" and msg.velocity > 0:
            if msg.channel == a.drums:
                s = t // slot
                lane = drum_lane(msg.note)
                if lane:
                    drum_hits[s].add(lane)
            else:
                open_notes[msg.channel][msg.note] = t
        elif msg.type == "note_off" or (msg.type == "note_on" and msg.velocity == 0):
            st = open_notes[msg.channel].pop(msg.note, None)
            if st is not None and msg.channel != a.drums:
                ch_notes[msg.channel].append((st // slot, max(t // slot, st // slot + 1), msg.note))
    if bpm_unset(bpm):
        bpm = 120.0

    total_slots = 0
    for evs in ch_notes.values():
        for _, e, _ in evs:
            total_slots = max(total_slots, e)
    total_slots = max(total_slots, (max(drum_hits) + 1) if drum_hits else 0)
    total_bars = (total_slots + bar_slots - 1) // bar_slots
    if a.max_bars:
        total_bars = min(total_bars, a.max_bars)

    # monophonic grids: melodic[channel] = list over slots of (note or None, is_attack)
    def grid(channel, pick_high):
        g = [None] * (total_bars * bar_slots)
        att = [False] * len(g)
        for s0, s1, n in sorted(ch_notes.get(channel, [])):
            for s in range(s0, min(s1, len(g))):
                cur = g[s]
                if cur is None or (n > cur if pick_high else n < cur):
                    g[s] = n
                    att[s] = (s == s0) or att[s]
            if s0 < len(g):
                att[s0] = True
        return g, att

    lead_g, lead_a = grid(a.lead, True)
    bass_g, bass_a = grid(a.bass, False)
    stab_g, stab_a = grid(a.stab, True)
    arp_g, arp_a = grid(a.arp, True) if a.arp >= 0 else (None, None)

    def lane_str(g, att):
        out = []
        for s in range(len(g)):
            if g[s] is None:
                out.append(".")
            elif att[s] and (s == 0 or g[s] != g[s - 1] or not contiguous(att, s)):
                out.append(note_name(g[s]))
            elif g[s] == g[s - 1]:
                out.append("-")
            else:
                out.append(note_name(g[s]))
        return out

    def contiguous(att, s):  # was the prev slot part of the same sustained note (no new attack here)
        return not att[s]

    lead = lane_str(lead_g, lead_a)
    bass = lane_str(bass_g, bass_a)
    arp = lane_str(arp_g, arp_a) if arp_g is not None else None

    def fmt_bar(tokens):  # 16 tokens → "x... .... .... ...." style groups of 4
        return "  ".join(" ".join(tokens[i:i + 4]) for i in range(0, 16, 4))

    # per-bar chord from the dominant bass root in that bar
    def bar_chord(b):
        roots = [bass_g[b * 16 + i] for i in range(16) if bass_g[b * 16 + i] is not None]
        if not roots:
            return None
        return chord_of(collections.Counter(roots).most_common(1)[0][0])

    L = []
    L.append("# martin score — transcribed from a public MIDI + quantised to martin's 16-slot grid by")
    L.append("# pipeline/transcribe_midi.py (NOT a from-memory guess). An instrumental cover arrangement")
    L.append("# on martin's synth voices; hand-tune the mix / sections / voice-switches after. Credit the")
    L.append("# original composition in REUSE.toml (see the camping / d2t scores).")
    L.append(f"bpm {bpm:g}")
    # a default chord set (overridden per-section below); fall back to Am if a bar has no bass
    L.append("chords Am G F E")
    L.append("")
    L.append("set lead=0.85 leadsw=5 arp=0.85 arpsw=1 bass=0.7 sub=0.5 stab=0.9 supersaw=0.5 choir=0.45 reverb=0.35 sidechain=0.5 hats=0.45 snares=0.55")
    # mids automation drives the pad/wall/stab body — ramp it up out of the intro so the bed fills
    L.append("")
    nsec = (total_bars + a.bars_per_section - 1) // a.bars_per_section
    for si in range(nsec):
        b0 = si * a.bars_per_section
        nb = min(a.bars_per_section, total_bars - b0)
        L.append(f"section s{si + 1} {nb} {nb}")
    L.append("")
    for si in range(nsec):
        name = f"s{si + 1}"
        b0 = si * a.bars_per_section
        nb = min(a.bars_per_section, total_bars - b0)
        sl0, sl1 = b0 * 16, (b0 + nb) * 16
        # chords per bar
        chs = [bar_chord(b0 + k) or "Am" for k in range(nb)]
        L.append(f"{name}.chords: " + " ".join(chs))
        # fill the bed: the supersaw/choir trance WALL + a sparkle, on every section past the intro
        # (the built-in `wall` default only fires on sections literally named drop/climax/outro).
        if si > 0:
            L.append(f"{name}.fx: wall shimmer house")
        # drums: DSL patterns are a SINGLE repeating 16-slot bar (not a multi-bar phrase) — collapse the
        # section's bars to the modal non-empty bar (the groove that recurs). Per-bar variation is lost;
        # that's the "transcribes notes, not structure" caveat — hand-add p1/fill afterwards.
        def modal_bar(cell_fn):
            pats = collections.Counter()
            for k in range(nb):
                bs = b0 + k
                cells = tuple("x" if cell_fn(bs * 16 + i) else "." for i in range(16))
                if "x" in cells:
                    pats[cells] += 1
            return list(pats.most_common(1)[0][0]) if pats else None

        for lane in ("kick", "snare", "hat"):
            cells = modal_bar(lambda s, ln=lane: ln in drum_hits.get(s, ()))
            if cells:
                L.append(f"{name}.{lane} p0: " + fmt_bar(cells))
        # melodic lanes
        lanes = [("lead", lead), ("bass", bass)]
        if arp is not None:
            lanes.insert(1, ("arp", arp))
        for lane, data in lanes:
            bars = [fmt_bar(data[(b0 + k) * 16:(b0 + k) * 16 + 16]) for k in range(nb)]
            if any(any(t not in (".", "-") for t in b.split()) for b in bars):
                L.append(f"{name}.{lane} p0: " + "   ".join(bars))
        # stab trigger from the chord channel (also a single repeating 16-pattern)
        scells = modal_bar(lambda s: stab_a[s] if s < len(stab_a) else False)
        if scells:
            L.append(f"{name}.stab p0: " + fmt_bar(scells))
        L.append("")

    open(a.out, "w").write("\n".join(L) + "\n")
    print(f"transcribed {total_bars} bars → {nsec} sections @ {bpm:g} BPM → {a.out}")


def bpm_unset(b):
    return not b


def drum_lane(note):
    if note in (35, 36):
        return "kick"
    if note in (38, 40, 37, 39):
        return "snare"
    if note in (42, 44, 46, 54, 69, 70):
        return "hat"
    return None


if __name__ == "__main__":
    sys.exit(main())
