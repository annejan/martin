#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
# SPDX-License-Identifier: MIT
"""MIDI → a complete, CLEAN martin score (the tracker DSL). One command turns a karaoke/GM song MIDI
into a martin `.txt` score that sounds like the song instead of mush — the lessons learned from the
jantje project (the user's MIDI→C64-SID arranger), applied to martin's own synth:

  * LEAD = the channel LABELLED the vocal (track name `Melody`/`Vocal`/`CANTO` or a lead GM patch) —
    NOT the highest-pitched one. A clean sustained lead reads as "the singing".
  * BASS = the labelled/low bass channel, on a clean bass voice.
  * DRUMS = the song's OWN groove, transcribed from the GM drum channel (kick/snare/hat) — not a
    generic four-on-the-floor bonk laid on top.
  * FILL = a signature riff channel dropped ONLY into the vocal's whole-bar rest holes (it answers her
    phrases); never crammed over a busy vocal.
  * per-bar chords from the bass so the pad/stab follow the harmony (no held-wrong-chord clash).
  * minimal + clean: lead + bass + drums (+ the gap-fill). Nothing else stacked, no brutal saws.

    pipeline/midi_to_martin.py song.mid out.txt                       # auto-detect everything
    pipeline/midi_to_martin.py song.mid out.txt --vocal 4 --bass 2 --fill 3 --bpm 130 --arrange song

Channels are 1-BASED on the CLI (matching jantje's midi_inspect display); internally 0-based.
Run with no out.txt to just PRINT the detected role map (a dry-run, like midi_inspect + midi_arrange).
"""
import argparse
import struct
import sys
from collections import Counter

NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]
# GM program families that read as a lead/vocal vs a bass.
LEAD_PROGS = set(range(64, 80)) | set(range(80, 88))  # reed/pipe(sax,flute) + synth-lead
BASS_PROGS = set(range(32, 40))                        # acoustic/electric/synth bass
VOCAL_WORDS = ("vocal", "melody", "canto", "lead", "voce", "sing", "vox", "voix")
BASS_WORDS = ("bass", "bas")


def _varlen(d, i):
    v = 0
    while True:
        b = d[i]; i += 1; v = (v << 7) | (b & 0x7F)
        if not b & 0x80:
            return v, i


def parse(path):
    """Full parse → (div, bpm, notes, meta). notes = [(start,end,pitch,chan)]; meta maps each channel
    to {name, program, n, lo, hi} (track name from FF 03 / FF 04, GM program from 0xC0)."""
    d = open(path, "rb").read()
    assert d[:4] == b"MThd", "not a MIDI"
    _fmt, ntrk, div = struct.unpack(">HHH", d[8:14])
    tempo = 500000
    tempos = []  # (abs_tick, usec_per_quarter) — the rubato curve, for the tempo map
    sigs = []    # (abs_tick, numerator, denominator) — time-signature changes (odd meter)
    notes = []
    chan_name, chan_prog = {}, {}
    pos = 14
    for _ in range(ntrk):
        if d[pos:pos + 4] != b"MTrk":
            break
        ln = struct.unpack(">I", d[pos + 4:pos + 8])[0]
        body = d[pos + 8:pos + 8 + ln]
        i = 0; t = 0; status = 0; on = {}; tname = None; tchans = set()
        while i < len(body):
            dt, i = _varlen(body, i); t += dt
            b = body[i]
            if b & 0x80:
                status = b; i += 1
            ev, ch = status & 0xF0, status & 0x0F
            if status == 0xFF:
                meta = body[i]; i += 1; mlen, i = _varlen(body, i)
                if meta in (0x03, 0x04) and tname is None:
                    tname = body[i:i + mlen].decode("latin-1", "replace").strip()
                elif meta == 0x51:
                    tempo = int.from_bytes(body[i:i + 3], "big")
                    tempos.append((t, tempo))
                elif meta == 0x58 and mlen >= 2:
                    sigs.append((t, body[i], 2 ** body[i + 1]))  # numerator, 2^denominator
                i += mlen
            elif status in (0xF0, 0xF7):
                mlen, i = _varlen(body, i); i += mlen
            elif ev == 0xC0:
                chan_prog[ch] = body[i]; tchans.add(ch); i += 1
            elif ev == 0x90:
                pitch, vel = body[i], body[i + 1]; i += 2; tchans.add(ch)
                if vel > 0:
                    on[(ch, pitch)] = t
                else:
                    s = on.pop((ch, pitch), None)
                    if s is not None:
                        notes.append((s, t, pitch, ch))
            elif ev == 0x80:
                pitch = body[i]; i += 2; s = on.pop((ch, pitch), None)
                if s is not None:
                    notes.append((s, t, pitch, ch))
            elif ev in (0xA0, 0xB0, 0xE0):
                i += 2
            elif ev == 0xD0:
                i += 1
            else:
                i += 1
        if tname:
            for c in tchans:
                chan_name.setdefault(c, tname)
        pos += 8 + ln
    bpm = round(60_000_000 / tempo)
    meta = {}
    for c in sorted({n[3] for n in notes}):
        ps = [p for s, e, p, ch in notes if ch == c]
        meta[c] = {"name": chan_name.get(c, ""), "program": chan_prog.get(c),
                   "n": len(ps), "lo": min(ps), "hi": max(ps), "avg": sum(ps) // len(ps)}
    return div, bpm, notes, meta, tempos, sigs


def tempo_curve(tempos, div, a, end, fallback_bpm):
    """From the MIDI's set-tempo events (the rubato curve) → (base_bpm, "tempo @bar:N=BPM ..." line).
    Bars are made OUTPUT-relative: the score's bar 0 is the song's first bar `a`, so a MIDI event at bar
    B emits `@bar:(B-a)`. The base is the tempo in effect AT the start (the last event with bar <= a).
    First-event-per-bar wins; a step that doesn't change the rounded BPM is dropped."""
    if not tempos:
        return fallback_bpm, ""
    tpb = div * 4
    per_bar = {}  # midi bar -> bpm (first event in the bar wins)
    for tick, usec in tempos:
        bar = tick // tpb
        per_bar.setdefault(bar, max(1, round(60_000_000 / usec)))
    base = fallback_bpm
    for bar in sorted(per_bar):  # tempo in effect at the song's start = last event at/before bar a
        if bar <= a:
            base = per_bar[bar]
    pts, last = [], base
    for bar in sorted(per_bar):
        if not a < bar < end:  # only steps inside the rendered span; bar a is the base
            continue
        if per_bar[bar] != last:
            pts.append((bar - a, per_bar[bar]))
            last = per_bar[bar]
    line = "tempo " + " ".join(f"@bar:{b}={v}" for b, v in pts) if pts else ""
    return base, line


def detect_roles(meta):
    """Pick (vocal, bass, drums, fill) channels from the metadata, jantje-style: the LABELLED vocal
    beats the highest one. Returns a dict of 0-based channels (any may be None)."""
    drums = 9 if 9 in meta else None
    mel = [c for c in meta if c != 9]

    def score_vocal(c):
        m = meta[c]; s = 0
        nm = m["name"].lower()
        if any(w in nm for w in VOCAL_WORDS):
            s += 1000                  # a LABELLED vocal always wins (the jantje rule)
        if m["program"] in LEAD_PROGS:
            s += 30
        s += min(m["avg"], 75) * 0.8   # prefer a high register, but CAP it — an extreme-high accent
        #                                (one note at 84) is not "more melody" than a mid-high line.
        if m["avg"] < 52:              # a melody isn't in the bass (a synth-bass pulse on a lead patch)
            s -= (52 - m["avg"]) * 2
        if m["n"] < 40:                # nor is it a 5-note accent — a real line is moderately active
            s -= (40 - m["n"]) * 2
        return s

    def score_bass(c):
        m = meta[c]; s = 0
        if any(w in m["name"].lower() for w in BASS_WORDS):
            s += 100
        if m["program"] in BASS_PROGS:
            s += 40
        s += (108 - m["avg"]) * 0.5   # low register
        return s

    vocal = max(mel, key=score_vocal) if mel else None
    bass = max([c for c in mel if c != vocal], key=score_bass, default=None)
    # fill = the busiest remaining MELODIC riff — but not a percussion patch (GM 112-119: agogo/
    # woodblock/taiko…) or an extreme-high ostinato, which read as clang, not a counter-melody.
    rest = [c for c in mel if c not in (vocal, bass)]
    musical = [c for c in rest if not 112 <= (meta[c]["program"] or 0) <= 119 and meta[c]["avg"] < 88]
    fill = max(musical or rest, key=lambda c: meta[c]["n"], default=None)
    return {"vocal": vocal, "bass": bass, "drums": drums, "fill": fill}


def auto_style(meta, has_drums):
    """Pick a `--style` palette from the MIDI's GM instrument families (note-weighted), so 80s synth
    songs render synthpop, a sax/brass combo jazz, a distorted-guitar band rock, drumless classical
    orchestral — no manual --style guess. Returns a style name (default `clean`)."""
    fam = Counter()
    for c, m in meta.items():
        if c == 9:
            continue
        p = m["program"] or 0
        n = m["n"]
        if 29 <= p <= 31:
            fam["distguitar"] += n          # overdrive / distortion guitar → rock
        elif p in (50, 51, 62, 63) or 80 <= p <= 95:
            fam["synth"] += n               # SynthStrings / SynthBrass + synth lead/pad (the 80s tell)
        elif 56 <= p <= 61:
            fam["brass"] += n               # real trumpet / trombone / brass section
        elif 64 <= p <= 67:
            fam["sax"] += n                 # sax (the jazz / Pink-Panther tell)
        elif 68 <= p <= 79:
            fam["wind"] += n                # oboe / clarinet / flute
        elif 40 <= p <= 49 or 52 <= p <= 55:
            fam["strings"] += n             # real strings + ensemble / choir / orchestra hit
    total = sum(fam.values()) or 1
    frac = lambda k: fam[k] / total
    orch = frac("strings") + frac("wind") + frac("brass")
    if has_drums and frac("distguitar") > 0.12:
        return "rock"
    if has_drums and fam["sax"] > 0 and frac("sax") + frac("brass") > 0.10:
        return "jazz"
    if frac("synth") > 0.30:
        return "synthpop"
    if orch > 0.45 and frac("synth") < 0.15:
        return "orchestral"                  # an orchestration (even with a light kit) → lush + soft
    if not has_drums:
        return "orchestral"                  # solo piano / ambient classical → lush + soft
    return "clean"


# --- note-grid (16 slots/bar; mono: highest for lead, lowest for bass) -----------------------------
def grid(notes, div, chan, lane, lo, hi):
    spb = max(div // 4, 1)
    sel = [n for n in notes if n[3] == chan and lo <= n[2] <= hi]
    last = max((e for s, e, p, c in sel), default=0)
    nsl = (((last // spb) + 16) // 16) * 16
    starts = {}
    for s, e, p, c in sel:
        sl = round(s / spb)
        if sl >= nsl:
            continue
        cur = starts.get(sl)
        if cur is None or (p > cur[0] if lane == "lead" else p < cur[0]):
            starts[sl] = (p, e)
    out = [None] * nsl; held = -1; hs = None
    for sl in range(nsl):
        if sl in starts:
            p, e = starts[sl]; out[sl] = ("n", p); held = round(e / spb); hs = sl
        elif hs is not None and sl < held:
            out[sl] = ("t",)
    return out


def tok(x):
    if x is None:
        return "."
    if x[0] == "t":
        return "-"
    p = x[1]
    return f"{NOTE_NAMES[p % 12]}{p // 12 - 1}"


def bars_of(slots):
    return ["  ".join(" ".join(tok(slots[b + g + k] if b + g + k < len(slots) else None)
                                for k in range(4)) for g in range(0, 16, 4))
            for b in range(0, len(slots), 16)]


FOUR_ON_FLOOR = {"kick": "x... x... x... x...", "snare": ".... x... .... x...",
                 "hat": "..x. ..x. ..x. ..xx"}


def drum_bar(notes, div, a, b):
    """One representative 1-bar kick/snare/hat pattern from GM channel 9, accumulated over bars [a,b).
    Falls back to a canonical four-on-the-floor when the source has no usable drum channel."""
    spb = max(div // 4, 1); tpb = div * 4
    sets = {"kick": {35, 36}, "snare": {38, 40, 37}, "hat": {42, 44, 46, 39, 54}}
    out = {}
    for kind, ps in sets.items():
        row = ["."] * 16
        for s, e, p, c in notes:
            if c == 9 and p in ps and a * tpb <= s < b * tpb:
                sl = round((s % tpb) / spb)
                if 0 <= sl < 16:
                    row[sl] = "x"
        out[kind] = " ".join("".join(row[i:i + 4]) for i in range(0, 16, 4))
    if "x" not in out["kick"]:  # no real drums → lay a house floor so it still grooves
        return dict(FOUR_ON_FLOOR)
    return out


def bar_chord(notes, div, chan, bi, prev):
    tpb = div * 4
    bn = [p for s, e, p, c in notes if c == chan and bi * tpb <= s < (bi + 1) * tpb]
    if not bn:
        return prev or "C"
    r = min(bn) % 12; pcs = {p % 12 for p in bn}
    minor = (r + 3) % 12 in pcs and (r + 4) % 12 not in pcs
    return NOTE_NAMES[r] + ("m" if minor else "")


# Voice/mix palettes (the `set` line). Genre-appropriate instrumentation beats one fixed voice set —
# validated by ear (Never Ending Story / You Spin Me Round wanted the 80s palette, not the clean one).
STYLES = {
    "clean": "set lead=0.78 leadsw=0 arp=0.42 arpsw=2 basssw=2 sub=0.5 supersaw=0.1 choir=0.06 padsw=5 "
             "stabsw=1 donk=0 house=0 reverb=0.32 sidechain=0.55 widen=1.7 makeup=1.14 ceiling=0.95 "
             "shimmer=0.08 hats=0.4 snares=0.48 atmosphere=0.06 drumsw=1 oversample=1",
    "synthpop": "set lead=0.84 leadsw=1 arp=0.42 arpsw=3 basssw=2 sub=0.46 supersaw=0.3 choir=0.22 "
                "padsw=2 stabsw=1 donk=0.1 house=0.08 reverb=0.44 sidechain=0.4 widen=1.95 makeup=1.12 "
                "ceiling=0.95 shimmer=0.28 hats=0.4 snares=0.52 atmosphere=0.05 drumsw=2 oversample=1",
    "dance": "set lead=0.8 leadsw=0 arp=0.5 arpsw=5 basssw=4 sub=0.5 supersaw=0.18 choir=0.0 padsw=5 "
             "stabsw=2 donk=0.16 house=0.14 reverb=0.32 sidechain=0.78 widen=1.8 makeup=1.16 "
             "ceiling=0.94 shimmer=0.12 hats=0.42 snares=0.5 atmosphere=0.08 drumsw=1 oversample=1",
    "rock": "set lead=0.85 leadsw=4 arp=0.3 arpsw=5 basssw=3 sub=0.5 supersaw=0.12 choir=0.0 padsw=5 "
            "stabsw=2 donk=0 house=0 reverb=0.28 sidechain=0.35 widen=1.6 makeup=1.16 ceiling=0.94 "
            "shimmer=0.04 hats=0.42 snares=0.56 atmosphere=0.04 drumsw=0 oversample=1",
    "orchestral": "set lead=0.72 leadsw=0 arp=0.36 arpsw=2 basssw=2 sub=0.5 supersaw=0.26 choir=0.3 "
                  "padsw=4 stabsw=0 donk=0 house=0 reverb=0.52 sidechain=0.2 widen=1.8 makeup=1.1 "
                  "ceiling=0.95 shimmer=0.18 hats=0.3 snares=0.42 atmosphere=0.1 drumsw=1 oversample=1",
    # jazz: breathy sax-ish lead, vibes-glass arp, finger/upright bass, soft brushy kit, warm room,
    # no acid/supersaw/pump. Pair with a `swing` directive for the long-short feel (set --swing).
    "jazz": "set lead=0.88 leadsw=2 arp=0.3 arpsw=3 basssw=2 sub=0.42 supersaw=0.04 choir=0.03 padsw=5 "
            "stabsw=1 donk=0 house=0 reverb=0.44 sidechain=0.22 widen=1.7 makeup=1.12 ceiling=0.95 "
            "shimmer=0.05 hats=0.3 snares=0.36 atmosphere=0.04 drumsw=1 oversample=1",
}


def style_set(args, faithful):
    """The `set` line for the chosen --style; faithful mode softens the pump (natural, not a club mix)."""
    s = STYLES.get(args.style, STYLES["clean"])
    return s.replace("sidechain=0.55", "sidechain=0.32") if faithful and args.style == "clean" else s


# Arrangement presets: (section_name, role, bars). Roles drive the drums/voice mix.
ARRANGES = {
    "song": [("intro", "intro", 8), ("v1", "verse", 16), ("c1", "chorus", 16),
             ("v2", "verse", 16), ("c2", "peak", 16), ("out", "out", 8)],
    "short": [("intro", "intro", 8), ("verse", "verse", 16), ("chorus", "peak", 16), ("out", "out", 8)],
    "dance": [("intro", "intro", 8), ("build", "build", 8), ("drop", "chorus", 16),
              ("break", "verse", 8), ("peak", "peak", 16), ("out", "out", 8)],
}


def empty_bar(b):
    return all(t in (".", "-") for t in b.split())


def _faithful(out, args, div, bpm, notes, roles, voc, bas, fil, a, tempos=None, suppress=False):
    """A reasonably TRUE rendition: the song played THROUGH once at its own tempo with all the main
    parts — lead + bass + a harmony voice + the real drum groove — in ONE long section, instead of a
    looped 16-bar dance remix with a four-on-the-floor laid over it. Natural mix (no heavy pump)."""
    tpb = div * 4
    a = min((s for s, e, p, c in notes), default=0) // tpb  # the SONG's start, not the lead's entry —
    #   else featuring a late-entering lead (an atmospheric harp) would lop off the whole intro.
    end = max(e for s, e, p, c in notes) // tpb + 1
    nb = max(end - a, 1)
    # the rubato tempo map (output-bar-relative). Suppressed by --bpm / --no-tempo-map / a misread tempo.
    bpm, tline = (bpm, "") if suppress else tempo_curve(tempos, div, a, end, bpm)
    split = bas is None
    bsrc = voc if split else bas
    # capture the lead channel's OWN register (else a low-register lead, e.g. a synth-bass pulse the
    # detector mistook for the tune, falls entirely below a hardcoded floor → an EMPTY lead).
    if split:
        lead_lo, lead_hi, bass_hi = 55, 104, 54
    else:
        pr = [p for s, e, p, c in notes if c == voc]
        lead_lo, lead_hi = (min(pr), max(pr)) if pr else (48, 100)
        bass_hi = 60
    pad = lambda arr: [(arr[i] if i < len(arr) else ". . . .  . . . .  . . . .  . . . .") for i in range(nb)]
    LEAD = pad(bars_of(grid(notes, div, voc, "lead", lead_lo, lead_hi))[a:end])
    BASS = pad(bars_of(grid(notes, div, bsrc, "bass", 21, bass_hi))[a:end])
    HARM = pad(bars_of(grid(notes, div, fil, "lead", 45, 100))[a:end]) if fil is not None else None
    DR = drum_bar(notes, div, a + 8, a + 16)  # a representative groove (1-bar loop)
    CH = []; prev = None
    for bi in range(a, a + nb):
        prev = bar_chord(notes, div, bsrc, bi, prev); CH.append(prev)
    sw = swing_value(args, notes, div, voc)
    L = [f'# {args.title or "song"} — FAITHFUL rendition (midi_to_martin.py --faithful): the song through',
         "# once at its own tempo — lead + bass + harmony + the real drum groove, natural mix (no pump).",
         f"bpm {bpm}",
         *( [f"swing {sw}"] if sw > 0 else [] ),
         *( [tline] if tline else [] ),
         "chords " + " ".join(CH),
         style_set(args, True),
         "", f"section song {nb} {nb}", ""]
    if not args.no_drums:  # solo piano / ambient sources have no drums + don't want a house floor over them
        L += [f"song.kick p0: {DR['kick']}", f"song.snare p0: {DR['snare']}", f"song.hat p0: {DR['hat']}", ""]
    L.append("song.lead p0: " + "  ".join(x.strip() for x in LEAD))
    if HARM:
        L.append("song.arp p0: " + "  ".join(x.strip() for x in HARM))
    L.append("song.bass p0: " + "  ".join(x.strip() for x in BASS))
    open(out, "w").write("\n".join(L) + "\n")
    rn = lambda c: f"ch{c + 1}" if c is not None else "-"
    print(f"wrote {out}: FAITHFUL, bpm {bpm}, {nb} bars | vocal={rn(voc)} "
          f"bass={rn(bsrc)} harm={rn(fil)} drums={rn(roles['drums'])}")


def has_odd_meter(sigs):
    """True if the MIDI carries a non-4/4 time signature (so it needs the variable-grid renderer)."""
    return any((n, d) != (4, 4) for _t, n, d in sigs)


def detect_swing(notes, div, chan):
    """Estimate swing (0..1) from how late the off-8th onsets land vs a straight grid, for channel
    `chan`. Looks at notes whose nearest 16th is the off-8th (slot 2 of a beat); the median within-beat
    fraction r_obs maps to swing = clamp(6*(r_obs-0.5), 0, 1). Too few off-8ths → 0 (straight)."""
    spb = max(div // 4, 1)
    beat = 4 * spb
    fracs = []
    for s, e, p, c in notes:
        if c != chan:
            continue
        sl = round(s / spb) % 4
        if sl == 2:  # the "and" of a beat — the slot swing pushes
            fracs.append((s % beat) / beat)
    if len(fracs) < 8:
        return 0.0
    fracs.sort()
    r_obs = fracs[len(fracs) // 2]  # median within-beat fraction of the off-8ths
    return max(0.0, min(1.0, 6.0 * (r_obs - 0.5)))


def swing_value(args, notes, div, lead_chan):
    """The swing amount to emit: --swing wins; else --auto-swing estimates it from the lead channel."""
    if args.swing and args.swing > 0:
        return max(0.0, min(1.0, args.swing))
    if getattr(args, "auto_swing", False):
        sw = detect_swing(notes, div, lead_chan)
        if sw > 0.05:
            print(f"  (auto-swing: detected ~{sw:.2f} from off-8th timing)", file=sys.stderr)
            return round(sw, 2)
    return 0.0


def bars_meter(sigs, end_tick, div):
    """Per-bar (start_16th_slot, grid) covering [0, end_tick), from the time-signature map. A slot is a
    16th (div/4 ticks); a num/den bar is `num*16/den` slots and `num*4*div/den` ticks (3/4 → 12, 4/4 → 16)."""
    spb = max(div // 4, 1)
    sigs = sorted(sigs) if sigs else [(0, 4, 4)]
    bars, si, t = [], 0, 0
    while t < end_tick and len(bars) < 4000:
        while si + 1 < len(sigs) and sigs[si + 1][0] <= t:
            si += 1
        num, den = sigs[si][1], sigs[si][2]
        bars.append((t // spb, max(1, num * 16 // den)))
        t += max(spb, num * 4 * div // den)
    return bars


def fmt_slots(slots):
    """Format a bar's slots (note tuples) as tokens grouped in 4s with a double space between groups."""
    t = [tok(x) for x in slots]
    return "  ".join(" ".join(t[i:i + 4]) for i in range(0, len(t), 4))


def drum_meter(notes, div, start16, g):
    """A `g`-slot kick/snare/hat pattern for the bar starting at absolute 16th `start16`."""
    spb = max(div // 4, 1)
    sets = {"kick": {35, 36}, "snare": {38, 40, 37}, "hat": {42, 44, 46, 39, 54}}
    out = {}
    for kind, ps in sets.items():
        row = ["."] * g
        for s, e, p, c in notes:
            if c == 9 and p in ps:
                sl = round(s / spb) - start16
                if 0 <= sl < g:
                    row[sl] = "x"
        out[kind] = "  ".join("".join(row[i:i + 4]) for i in range(0, g, 4))
    return out


def chord_in(notes, div, chan, lo16, hi16, prev):
    """Chord name from the `chan` notes whose onset 16th is in [lo16, hi16) — root + minor heuristic."""
    spb = max(div // 4, 1)
    bn = [p for s, e, p, c in notes if c == chan and lo16 <= round(s / spb) < hi16]
    if not bn:
        return prev or "C"
    r = min(bn) % 12
    pcs = {p % 12 for p in bn}
    minor = (r + 3) % 12 in pcs and (r + 4) % 12 not in pcs
    return NOTE_NAMES[r] + ("m" if minor else "")


def _faithful_meter(out, args, div, bpm, notes, roles, voc, bas, fil, sigs, tempos=None, suppress=False):
    """A FAITHFUL rendition of an ODD-METER piece (Golden Brown's 3/4↔4/4): one section PER BAR, each
    with `grid:N` from the time signature, so the meter is real instead of force-fitted to 4/4. The
    rubato tempo map composes on top (one section per bar ⇒ score bar k = MIDI bar k)."""
    spb = max(div // 4, 1)
    end_tick = max(e for s, e, p, c in notes)
    bars = bars_meter(sigs, end_tick, div)
    # tempo map over the meter bars (so a rubato compound-meter piece — Clair de Lune in 9/8 — keeps its
    # breathing): map each set-tempo tick → the meter bar it lands in, base = bar-0 tempo.
    tline = ""
    if not suppress and tempos:
        starts16 = [s for s, _g in bars] + [bars[-1][0] + bars[-1][1]]
        per_bar = {}
        for tk, usec in tempos:
            t16 = tk // spb
            k = next((j for j in range(len(bars)) if starts16[j] <= t16 < starts16[j + 1]), 0)
            per_bar.setdefault(k, max(1, round(60_000_000 / usec)))
        bpm = per_bar.get(0, bpm)
        pts, last = [], bpm
        for k in sorted(per_bar):
            if k != 0 and per_bar[k] != last:
                pts.append((k, per_bar[k]))
            last = per_bar[k]
        tline = "tempo " + " ".join(f"@bar:{k}={v}" for k, v in pts) if pts else ""
    split = bas is None
    bsrc = voc if split else bas
    if split:
        lead_lo, lead_hi, bass_hi = 55, 104, 54
    else:
        pr = [p for s, e, p, c in notes if c == voc]
        lead_lo, lead_hi = (min(pr), max(pr)) if pr else (48, 100)
        bass_hi = 60
    flat_lead = grid(notes, div, voc, "lead", lead_lo, lead_hi)
    flat_bass = grid(notes, div, bsrc, "bass", 21, bass_hi)
    flat_harm = grid(notes, div, fil, "lead", 45, 100) if fil is not None else None
    slc = lambda flat, s, g: [flat[s + j] if s + j < len(flat) else None for j in range(g)]
    chords, prev = [], None
    for s, g in bars:
        prev = chord_in(notes, div, bsrc, s, s + g, prev)
        chords.append(prev)
    sw = swing_value(args, notes, div, voc)
    L = [
        f'# {args.title or "song"} — FAITHFUL odd-meter (midi_to_martin.py): one section per bar,',
        "# grid:N per the time signature so 3/4↔4/4 (Golden Brown) plays in its real meter.",
        f"bpm {bpm}",
        *([f"swing {sw}"] if sw > 0 else []),
        *([tline] if tline else []),
        "chords " + " ".join(chords),
        style_set(args, True),
        "",
    ]
    for k, (s, g) in enumerate(bars):
        L.append(f"section b{k} 1 grid:{g}")
    L.append("")
    for k, (s, g) in enumerate(bars):
        nm = f"b{k}"
        L.append(f"{nm}.lead p0: {fmt_slots(slc(flat_lead, s, g))}")
        if flat_harm is not None:
            L.append(f"{nm}.arp p0: {fmt_slots(slc(flat_harm, s, g))}")
        L.append(f"{nm}.bass p0: {fmt_slots(slc(flat_bass, s, g))}")
        if not args.no_drums:
            d = drum_meter(notes, div, s, g)
            for kind in ("kick", "snare", "hat"):
                if "x" in d[kind]:
                    L.append(f"{nm}.{kind} p0: {d[kind]}")
    open(out, "w").write("\n".join(L) + "\n")
    rn = lambda c: f"ch{c + 1}" if c is not None else "-"
    metres = sorted({g for _s, g in bars})
    print(f"wrote {out}: FAITHFUL ODD-METER, bpm {bpm}, {len(bars)} bars, grids {metres} | "
          f"vocal={rn(voc)} bass={rn(bsrc)} harm={rn(fil)}")


def build_score(path, out, args):
    div, bpm, notes, meta, tempos, sigs = parse(path)
    roles = detect_roles(meta)
    for k in ("vocal", "bass", "fill"):
        v = getattr(args, k)
        if v is not None:
            roles[k] = v - 1  # CLI is 1-based
    # an override can collide with the auto-picked fill (e.g. --vocal 4 when fill was 4) → re-pick a
    # distinct fill from the remaining melodic channels so the gap-fill isn't a no-op/double.
    if roles["fill"] is not None and roles["fill"] in (roles["vocal"], roles["bass"]):
        rest = [c for c in meta if c != 9 and c not in (roles["vocal"], roles["bass"])]
        roles["fill"] = max(rest, key=lambda c: meta[c]["n"], default=None)
    # auto-style: pick the palette from the GM instrument families unless the user forced one.
    if args.style == "auto":
        args.style = auto_style(meta, roles["drums"] is not None)
        print(f"  (auto-style: {args.style})", file=sys.stderr)
    if out is None:  # dry run — print the role map
        print(f"{path}: bpm {bpm}, div {div}, style {args.style}")
        for c in sorted(meta):
            m = meta[c]; tag = next((r for r, ch in roles.items() if ch == c), "")
            print(f"  ch{c + 1:2} {m['name'][:18]:18} prog={m['program']} n={m['n']:5} "
                  f"pitch {m['lo']}-{m['hi']:<3} {('<-- ' + tag.upper()) if tag else ''}")
        return
    bpm = args.bpm or bpm
    forced = args.bpm is not None  # a forced single tempo suppresses the rubato map
    if not 40 <= bpm <= 250:  # a misread tempo meta (the sonata read 25) → a sane default
        print(f"  (tempo {bpm} looks wrong — using 120; pass --bpm to set it)", file=sys.stderr)
        bpm = 120
        forced = True
    suppress_tempo = forced or args.no_tempo_map  # a forced/misread tempo → one steady tempo, no map
    voc, bas, fil = roles["vocal"], roles["bass"], roles["fill"]
    tpb = div * 4
    a = min((s for s, e, p, c in notes if c == voc), default=0) // tpb  # vocal's first bar
    if args.faithful:
        # an odd-meter source (non-4/4 time signature) → the variable-grid renderer, unless overridden.
        if has_odd_meter(sigs) and not args.no_meter:
            return _faithful_meter(
                out, args, div, bpm, notes, roles, voc, bas, fil, sigs, tempos, suppress_tempo
            )
        return _faithful(out, args, div, bpm, notes, roles, voc, bas, fil, a, tempos, suppress_tempo)
    # No separate bass channel (a single-channel piano piece) → SPLIT the lead channel by pitch: the
    # left hand (low notes) becomes the bass, the right hand (high) stays the lead.
    split = bas is None
    bsrc = voc if split else bas
    if split:
        lead_lo, lead_hi, bass_hi = 55, 96, 54
    else:  # capture the lead channel's own register (never an empty lead)
        pr = [p for s, e, p, c in notes if c == voc]
        lead_lo, lead_hi = (min(pr), max(pr)) if pr else (48, 96)
        bass_hi = 60
    VOC = bars_of(grid(notes, div, voc, "lead", lead_lo, lead_hi))[a:a + 16]
    BAS = bars_of(grid(notes, div, bsrc, "bass", 24, bass_hi))[a:a + 16]
    TIM = bars_of(grid(notes, div, fil, "lead", 40, 96))[a:a + 16] if fil is not None else None
    while len(VOC) < 16: VOC.append(". . . .  . . . .  . . . .  . . . .")
    while len(BAS) < 16: BAS.append(". . . .  . . . .  . . . .  . . . .")
    FILL = ([f if empty_bar(v) else ". . . .  . . . .  . . . .  . . . ."
             for v, f in zip(VOC, (TIM + ["."] * 16)[:16])] if TIM else None)
    DR = drum_bar(notes, div, a + 4, a + 12)
    CH = []; prev = None
    for bi in range(a, a + 16):
        prev = bar_chord(notes, div, bsrc, bi, prev); CH.append(prev)

    def sl(arr, n):
        return [arr[i % len(arr)] for i in range(n)]

    secs = ARRANGES.get(args.arrange, ARRANGES["song"])
    allch = []; lines = []
    for n, r, nb in secs:
        fill = r in ("chorus", "peak", "build")
        tot = nb + 1 if fill else nb
        lines.append((n, r, nb, tot)); c = sl(CH, nb); allch += c + ([c[-1]] if fill else [])

    L = []
    L.append(f'# {args.title or path} — clean martin score (pipeline/midi_to_martin.py). LEAD = the')
    L.append("# labelled vocal, clean bass, drums transcribed from the source, riff fills the vocal's gaps.")
    L.append(f"bpm {bpm}")
    L.append("chords " + " ".join(allch))
    L.append(style_set(args, False))
    L.append("")
    for n, r, nb, tot in lines:
        L.append(f"section {n} {tot} {nb} fill" if r in ("chorus", "peak", "build")
                 else f"section {n} {nb} {nb}")
    L.append("")
    for n, r, nb, tot in lines:
        L.append(f"{n}.kick p0: {DR['kick']}")
        L.append(f"{n}.snare p0: {DR['snare']}")
        L.append(f"{n}.hat p0: {DR['hat'] if any(c == 'x' for c in DR['hat']) else '..x. ..x. ..x. ..xx'}")
        if r in ("chorus", "peak", "build"):
            L.append(f"{n}.kick fill: {DR['kick']}")
            L.append(f"{n}.snare fill: ..x. ..x. x.x. xxxx")
            L.append(f"{n}.hat fill: xxxx xxxx xxxx xxxx")
            L.append(f"{n}.fx: wall riser" + (" shimmer" if r == "peak" else ""))
        if r == "intro":
            L.append(f"{n}.set lead=0 arp=0")
        if r == "build":
            L.append(f"{n}.set lead=0.4")
            L.append(f"{n}.fx: riser")
        if r == "out":
            L.append(f"{n}.set reverb=0.5 arp=0")
    L.append("")
    for n, r, nb, tot in lines:
        if r != "intro":
            L.append(f"{n}.lead p0: " + "  ".join(x.strip() for x in sl(VOC, nb)))
    if FILL:
        L.append("")
        for n, r, nb, tot in lines:
            if r in ("verse", "chorus", "peak"):
                L.append(f"{n}.arp p0: " + "  ".join(x.strip() for x in sl(FILL, nb)))
    L.append("")
    for n, r, nb, tot in lines:
        L.append(f"{n}.bass p0: " + "  ".join(x.strip() for x in sl(BAS, nb)))

    open(out, "w").write("\n".join(L) + "\n")
    rn = lambda c: f"ch{c + 1}" if c is not None else "-"
    bass_tag = "ch{} (split)".format(bsrc + 1) if split else rn(bas)
    print(f"wrote {out}: bpm {bpm}, {len(lines)} sections | "
          f"vocal={rn(voc)} bass={bass_tag} drums={rn(roles['drums'])} fill={rn(fil)}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("midi")
    ap.add_argument("out", nargs="?", help="output .txt (omit to just print the detected role map)")
    ap.add_argument("--vocal", type=int, help="force the vocal channel (1-based)")
    ap.add_argument("--bass", type=int, help="force the bass channel (1-based)")
    ap.add_argument("--fill", type=int, help="force the fill/riff channel (1-based)")
    ap.add_argument("--bpm", type=int, help="override the tempo (forces one steady tempo, no rubato map)")
    ap.add_argument("--no-tempo-map", action="store_true",
                    help="ignore the MIDI's tempo changes (render at one steady tempo)")
    ap.add_argument("--no-meter", action="store_true",
                    help="ignore odd time signatures (force-fit the song to 4/4)")
    ap.add_argument("--arrange", default="song", choices=list(ARRANGES), help="section structure")
    ap.add_argument("--style", default="auto", choices=["auto", *STYLES],
                    help="voice/mix palette; `auto` (default) picks one from the GM instruments "
                         "(clean|synthpop|dance|rock|orchestral|jazz)")
    ap.add_argument("--no-drums", action="store_true",
                    help="omit the kit (for solo-piano / ambient sources — no house floor over them)")
    ap.add_argument("--faithful", action="store_true",
                    help="play the song THROUGH at its own tempo (lead+bass+harmony+real drums, natural "
                         "mix) instead of a looped dance remix")
    ap.add_argument("--title", help="score title comment")
    ap.add_argument("--swing", type=float, default=0.0,
                    help="swing 0..1 (0=straight, 1=triplet) — emits a `swing N` line for the jazz feel")
    ap.add_argument("--auto-swing", action="store_true",
                    help="estimate swing from how late the off-8th notes land in the MIDI")
    a = ap.parse_args()
    build_score(a.midi, a.out, a)


if __name__ == "__main__":
    main()
