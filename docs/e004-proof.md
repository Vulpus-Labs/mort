# E004 proof: parts from a song sheet, in REAPER

Ticket 0029. REAPER 7.67 on macOS, bridge `0.1.0+1b9c7cdbce`. The
project is a throwaway one in a scratch folder, `song.rpp` with its
sidecar `song.mort/`.

## The session

Built with `mort run docs/e004-build.kdl`:

- Pad: vxn-2b;
- Keys: silkscreen Poly-Kritter;
- Bass: vxn-1c;
- Drums: ReaSynDr;
- regions `verse` (bars 1–9) and `chorus` (9–17), at 120 bpm in 4/4.

The sheet is [e004-sheet.kdl](e004-sheet.kdl):

- a modal verse, D dorian for 8 bars;
- a chorus of changes, Gm9 C13 Fmaj9 B♭maj7♯11 Gm9 C13 Fmaj9 A7alt;
- a quartal pad;
- drop-2 keys playing bossa piano;
- a bass walking the keys' bass voice;
- bossa drums.

### A kit map for ReaSynDr, by measurement

ReaSynDr hides its note mapping. mort wrote notes 30–61 one per beat on
the Drums track, rendered them, and read the probe's frames. Only notes
32–35 and 48–51 sound. Their spectral centroids, about 50, 150, 500 and
120 Hz, suggest kick, snare, hat and a click or tom. The project's kit
map, [e004-reasyndr.kdl](e004-reasyndr.kdl), maps them to kick, snare,
hat_closed and side_stick, in `song.mort/patterns/`. That loads it as a
project pattern file, which tests user kit maps too.

### Found: `project.save` with a path saves a copy

`mort call project.save '{"path": …}'` wrote `song.rpp`, but REAPER kept
the open project untitled and dirty: `Main_SaveProjectEx` saves a copy,
not *Save as*. `mort compose write` needs a project with a path, so
REAPER was restarted on the saved file. Follow-up ticket 0030.

## `mort compose show`

```text
Pad  (quartal, 4 voices, C3..A4)
  1.1     Gm9 (G dorian)             E3 13th · A3 9th · D4 5th · G4 root
  2.1     C13 (C mixolydian)         E3 3rd · A3 13th · D4 9th · G4 5th
  3.1     Fmaj9 (F ionian)           E3 7th · A3 3rd · D4 13th · G4 9th
  4.1     Bbmaj7#11 (Bb lydian)      E3 11th · A3 7th · D4 3rd · G4 13th
  5.1     Gm9 (G dorian)             E3 13th · A3 9th · D4 5th · G4 root
  6.1     C13 (C mixolydian)         E3 3rd · A3 13th · D4 9th · G4 5th
  7.1     Fmaj9 (F ionian)           E3 7th · A3 3rd · D4 13th · G4 9th
  8.1     A7alt (A altered)          B#2 9th · F3 13th · Bb3 9th · D#4 11th
Keys  (drop-2, 4 voices, G3..D5)
  1.1     Gm9 (G dorian)             Bb3 3rd · F4 7th · G4 root · D5 5th
  2.1     C13 (C mixolydian)         Bb3 7th · E4 3rd · G4 5th · C5 root
  3.1     Fmaj9 (F ionian)           A3 3rd · E4 7th · F4 root · C5 5th
  4.1     Bbmaj7#11 (Bb lydian)      A3 7th · D4 3rd · F4 5th · Bb4 root
  5.1     Gm9 (G dorian)             G3 root · D4 5th · F4 7th · Bb4 3rd
  6.1     C13 (C mixolydian)         Bb3 7th · E4 3rd · G4 5th · C5 root
  7.1     Fmaj9 (F ionian)           A3 3rd · E4 7th · F4 root · C5 5th
  8.1     A7alt (A altered)          A3 root · C#4 3rd · G4 7th · Bb4 9th
Bass  (plays the bass voice of Keys)
Drums  (drums: bossa drums)
state 12
```

## Writes, in REAPER

```text
$ mort compose write
Pad        verse        written: new
Pad        chorus       written: new
Keys       verse        written: new
Keys       chorus       written: new
Bass       verse        written: new
Bass       chorus       written: new
Drums      verse        written: new
Drums      chorus       written: new
state 3
```

Items: Pad 4 notes in the verse; Keys 92; Bass 32 (a note a beat); Drums
116 on channel 10. One batch: the state count moved by one.

```text
$ mort compose write
Pad        verse        unchanged
Pad        chorus       unchanged
Keys       verse        unchanged
Keys       chorus       unchanged
Bass       verse        unchanged
Bass       chorus       unchanged
Drums      verse        unchanged
Drums      chorus       unchanged
state 3

# Keys: articulation "bossa piano" -> "block"
$ mort compose write
Pad        verse        unchanged
Pad        chorus       unchanged
Keys       verse        written: articulation changed
Keys       chorus       written: articulation changed
Bass       verse        unchanged
Bass       chorus       unchanged
Drums      verse        unchanged
Drums      chorus       unchanged
state 4

# Keys: back to "bossa piano"
$ mort compose write
Pad        verse        unchanged
Pad        chorus       unchanged
Keys       verse        written: articulation changed
Keys       chorus       written: articulation changed
Bass       verse        unchanged
Bass       chorus       unchanged
Drums      verse        unchanged
Drums      chorus       unchanged
state 5

# chorus bar 8: A7alt -> E7alt
$ mort compose write
Pad        verse        unchanged
Pad        chorus       written: harmony changed
Keys       verse        unchanged
Keys       chorus       written: harmony changed
Bass       verse        unchanged
Bass       chorus       written: harmony changed
Drums      verse        unchanged
Drums      chorus       unchanged
state 6

# In REAPER, a Lua script (not mort) sets the first Pad note's velocity to 50
$ mort compose write
Pad        verse        edited by hand; left alone (--force overwrites it)
Pad        chorus       unchanged
Keys       verse        unchanged
Keys       chorus       unchanged
Bass       verse        unchanged
Bass       chorus       unchanged
Drums      verse        unchanged
Drums      chorus       unchanged
state 7

$ mort compose write --force --part Pad --section verse
Pad        verse        written: edited by hand; overwritten (--force)
state 8

# chorus bar 8 back to A7alt
$ mort compose write
Pad        verse        unchanged
Pad        chorus       written: harmony changed
Keys       verse        unchanged
Keys       chorus       written: harmony changed
Bass       verse        unchanged
Bass       chorus       written: harmony changed
Drums      verse        unchanged
Drums      chorus       unchanged
state 9

$ mort undo
undid mort: compose
state 10

$ mort redo
redid mort: compose
state 11

```

The edit "by hand" was made by a Lua script run inside REAPER
(`MIDI_SetNote` on the first Pad note), not through mort.

## Everything sounds, and the mix analysis has views

`mort measure 1.1..17.1` measured all four tracks for 640 frames each,
the drums included, so ReaSynDr answers on channel 10. `mort findings`:

```text
verse 0.00–16.00s
  1.00 register_clash Pad (E3) and Drums (D#3) sustain a minor second apart low down, first at 0.00s: revoice one, or move it an octave
  0.73 masking        Bass and Drums compete at 80–160 Hz in MASTER: cut one there, or move one in register or pan
  0.63 masking        Keys and Bass compete at 125–630 Hz in MASTER: cut one there, or move one in register or pan
  0.50 mud            MASTER has 5.0 dB more 200–500 Hz than its tilt predicts: cut 200–500 Hz on Keys
chorus 16.00–32.00s
  1.00 register_clash Pad (A3) and Keys (A#3) sustain a minor second apart low down, first at 16.00s: revoice one, or move it an octave
  1.00 register_clash Pad (E3) and Drums (D#3) sustain a minor second apart low down, first at 16.00s: revoice one, or move it an octave
  0.77 mud            MASTER has 6.6 dB more 200–500 Hz than its tilt predicts: cut 200–500 Hz on Pad or Keys
  0.65 masking        Pad and Keys compete at 200–400 Hz in MASTER: cut one there, or move one in register or pan
  0.42 masking        Bass and Drums compete at 50–125 Hz in MASTER: cut one there, or move one in register or pan
9 findings
state 12
```

- **Real: parts clash.** Pad A3 against Keys B♭3 in the chorus. Each
  part is voice-led alone, and nothing keeps parts off each other's
  semitones. Follow-up ticket 0031.
- **A bug: drum notes counted as pitches.** `register_clash` reads the
  drum track's MIDI as pitched: D♯3 is ReaSynDr's side stick. Follow-up
  ticket 0032.

## Listening

To come.
