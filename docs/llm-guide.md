# mort: a guide for language models

You have a shell with the `mort` command. mort drives a REAPER session
that a person has open on the same machine: it reads and edits the
project, plays and renders it, measures the mix, and reports problems.
Read this guide before your first mort command. `mort <command> --help`
gives the exact options for any command.

The person is working in the same project. They can hear; you cannot.
You work from measurements, so report what you measured and ask them to
listen to what you changed.

## Contents

1. Before you start
2. Syntax
3. Output, errors and exit codes
4. Command reference
5. Working loops
6. Rules
7. Reading measurements
8. Composing from a song sheet
9. Known problems

## 1. Before you start

Run these, in order, at the start of a session:

```sh
mort status     # bridge version, project name, held, playing, state
mort tracks     # tracks, volumes, pans, flags (M muted, S soloed), FX, sends
mort map        # what has been measured, and what is stale
```

- Exit code 3 from `mort status` means REAPER is not running the bridge.
  Ask the person to start REAPER. Do not try to start it yourself unless
  asked.
- `held yes` means the person has turned on *mort: hold edits*. Every
  edit will fail with `busy`. Reads and measurements still work. Do not
  try to get round it: tell the person, and wait.
- A project called `untitled` has never been saved. Measurements are kept
  in memory only, and `mort compose write` refuses to run. Ask the person
  to save it if you need either.

## 2. Syntax

### Positions and ranges

| Text | Meaning |
|---|---|
| `9` or `9.1` | bar 9, beat 1 |
| `9.2` | bar 9, beat 2 (beats of that bar's time signature) |
| `9.2.240` | bar 9, beat 2, plus 240 ticks at 960 per quarter note |
| `12.5s` | 12.5 seconds from the project start |
| `+4b` | 4 quarter notes after the previous position |
| `+1.5s` | 1.5 seconds after the previous position |

A range is two positions joined by `..`: `9.1..17.1`, `9.1..+8b`. A
relative end counts from the start. Positions follow the project's tempo
map.

Section (region) names are not accepted as ranges, except by
`mort compose`. To find a section's range, read its start and end from
`mort summary` (seconds) or `mort call project.get` (regions in seconds),
and use seconds (`27.43s..68.57s`), or convert:

```sh
mort call time.convert '{"to_musical": [27.43, 68.57]}'
```

### Naming things

- Track: number (from 1, as `mort tracks` lists), exact name, part of a
  name, id, or `master`.
- FX: position in the chain (from 1), name, part of a name (`ReaEQ`,
  `Kritter`), or id.
- Parameter: number (from 0, as `mort params` lists) or name. If a name
  matches more than one parameter, mort lists them; use the number.

### Values

- Track and send volume: dB, `-6` or `-inf`.
- Pan: `C`, `L20`, `R35`, or a number from −1 (left) to 1 (right).
- Plugin parameters with `mort set`: the text the plugin displays
  (`-3 dB`, `2.4 kHz`, `Sawtooth`), or a value from 0 to 1 with
  `--normalised`. If the text is outside the parameter's range, the error
  gives the range. Read the value back after setting it (section 9).
- Automation points: `POSITION=VALUE`. Volume in dB, pan as above, FX
  parameters as 0–1 (or as display text with `--text`). Note the
  difference: `mort set` reads display text unless told `--normalised`,
  and `mort automate` reads 0–1 unless told `--text`.

### MIDI events

A note is `PITCH:START[:LENGTH[:VELOCITY[:CHANNEL]]]`:

- pitch: a name (`C4` is middle C, 60; `F#2`, `Bb3`) or a number;
- start: a position; `+1b` is a quarter note after the previous event's
  start, so `+0b` stacks a chord (the first event needs a bar and beat);
- length: `1b` (quarter notes, the default) or `0.5s`;
- velocity 1–127, default 100; channel 1–16, default 1.

A CC is `ccN[@CHANNEL]:START=VALUE`, for example `cc1:9.1=64`.

The range says where to write: mort writes into a MIDI item that covers
it, and creates one over the range if there is none. With `--replace`,
notes starting in the range (and CCs in it) are cleared first.

```sh
mort midi Bass 9.1..17.1 C2:9.1:1b:100 E2:+1b:1b G2:+1b
mort midi Bass 9.1..17.1 --replace C2:9.1:4b      # clear the range first
mort midi Pad 9.1..17.1 --file chords.mid         # place a MIDI file at 9.1
```

### Scripts

`mort run FILE.kdl` (or `-` for stdin) runs one mort command per line.
The node's name and arguments are the command's words; a property
`key=value` becomes `--key value`, and `key=#true` becomes `--key`.
Consecutive edits are held and sent as one batch with one undo point. A
`commit` line sends the held batch; so does any step that needs the edits
made (measure, play, render, stop, undo). If a step fails, the script
stops there, names the step, and does not send its held edits.

```kdl
track add Pad
fx add Pad ReaSynth
set Pad ReaSynth Attack 0.5 normalised=#true
set Pad ReaSynth Volume "-12dB"
track set Pad volume=-12
send add Pad Verb
commit
send set Pad Verb volume=-10
measure "9.1..17.1" stale=#true
findings
```

Note the command line spells it `--volume -12`; a script spells it
`volume=-12`. Quote any value that is not a plain word or number
(`"-12dB"`, ranges). Steps can use tracks and FX added earlier in the
same batch, but not sends: the `commit` above is there so `send set` can
find the new send (section 9).

## 3. Output, errors and exit codes

- Text output is short and aligned. Most commands end with `state N`, the
  project's change counter. mort sends the state it read with each edit,
  so an edit fails with `stale` if the project changed in between; you do
  not pass it yourself.
- `--json` prints the bridge's raw result. Use it when you need ids or
  exact numbers to feed into the next command.
- Exit codes: 0 success; 1 an error from the bridge (or a local failure);
  2 a usage error; 3 no bridge running.
- An error from the bridge prints its message, and with `--json` also a
  `kind` (`not_found`, `stale`, `invalid`, `busy` or `reaper`), the
  `subject` it is about, and a `fix`: one sentence saying what to do.
  Follow the fix.
- `stale`: the project changed since you read it, usually because the
  person edited it. Read again (`mort tracks`, or whatever you were
  acting on), decide again, then retry.
- `busy`: edits are held. Stop editing and tell the person.
- `reaper`: a REAPER call failed. Read the state of the thing involved,
  try once more, and if it fails again tell the person.

## 4. Command reference

Run `mort <command> --help` for every option.

### Session

| Command | Does |
|---|---|
| `mort status` | bridge, project, held, playing, jobs, state |
| `mort call METHOD [JSON]` | any bridge RPC, for things with no command yet. Methods are registered in `crates/mort-bridge/src/methods.rs` in the mort repository; useful ones include `project.get` (project and regions), `time.convert`, `job.status` and `job.wait` |

### Reading

| Command | Does |
|---|---|
| `mort tracks` | every track: volume, pan, flags, FX, sends; folders indented |
| `mort fx TRACK` | the FX chain on a track |
| `mort params TRACK FX [FILTER]` | a plugin's parameters, values and display text |
| `mort midi TRACK [RANGE]` | MIDI items, notes and CCs |
| `mort automate TRACK TARGET` | an envelope's points; TARGET is `volume`, `pan`, `send:<track>` or `<fx>/<param>` |
| `mort map` | captures, coverage and stale ranges per track |

### Editing

Each edit is one undo point.

| Command | Does |
|---|---|
| `mort track add NAME [--at N] [--in FOLDER]` | add a track |
| `mort track rm TRACK` | remove a track |
| `mort track set TRACK [--name] [--volume] [--pan] [--mute] [--solo]` | change a track |
| `mort send add FROM TO` / `mort send set FROM TO [--volume] [--pan] [--mute]` | sends |
| `mort region add NAME RANGE` | add a region (a marker if given a position) |
| `mort fx add TRACK PLUGIN [--at N]` | add a plugin by the name REAPER's FX browser shows |
| `mort fx rm TRACK FX`, `fx bypass TRACK FX`, `fx enable TRACK FX` | remove, bypass or re-enable a plugin |
| `mort fx move TRACK FX TO` / `mort fx preset TRACK FX PRESET` | move a plugin in the chain; load a preset by name |
| `mort set TRACK FX PARAM VALUE [--normalised]` | set a plugin parameter |
| `mort automate TRACK TARGET POS=VALUE… [--shape S] [--add] [--clear RANGE]` | write automation; by default replaces the points between the first and last new one. Shapes: `linear`, `square`, `slow` (S-curve), `fast-start`, `fast-end`, `bezier` |
| `mort midi TRACK RANGE EVENTS… [--replace] [--file]` | write MIDI |
| `mort run FILE` | a script, as above |
| `mort undo` / `mort redo` | undo or redo mort's last edit; refuses if the last edit was not mort's |

### Playing

| Command | Does |
|---|---|
| `mort play RANGE` | play through the speakers, stopping at the end |
| `mort render RANGE` | render faster than real time without writing a file |
| `mort stop` | stop |

These wait for the job to finish. `--no-wait` prints the job id instead;
wait for it later with `mort call job.wait '{"id": N, "timeout_ms": 60000}'`.
`mort render` stores nothing; to measure a range, use `mort measure`.

### Measuring

| Command | Does |
|---|---|
| `mort probes` / `mort probes ensure [TRACKS]` | list probes, or put one last on each track and the master |
| `mort measure RANGE [TRACKS…] [--play]` | measure a range into the map; adds probes where needed; renders unless `--play` |
| `mort measure --stale [RANGE]` | re-measure only what is stale, with the buses it reaches |
| `mort summary [RANGE]` | per section: master loudness, peak, tilt, width, density; each track's level and share |
| `mort findings [RANGE] [--kind K] [--all] [--limit N]` | problems, worst first, with remedies. Kinds: `masking`, `mud`, `harsh`, `headroom`, `loudness_jump`, `low_end_width`, `register_clash` (sustained low notes a semitone apart, read from MIDI) |
| `mort thresholds` / `thresholds set KEY VALUE` / `thresholds reset KEY` | finding thresholds for this project; `mort thresholds` lists the keys, values and the reason for each default |
| `mort report TRACK [RANGE]` | one track per section: loudness, peak, crest, centroid, width, loudest bands, share of its bus |
| `mort frames TRACK RANGE [-f FEATURES] [--step S]` | raw features over time, one row per `S` seconds. Features: `momentary_lufs`, `short_term_lufs`, `sample_peak_dbfs`, `true_peak_dbtp`, `rms_dbfs`, `crest_db`, `correlation`, `correlation_low`, `centroid_hz`, `flatness`, `onset`, `band:<Hz>`, `side:<Hz>` |
| `mort compare [RANGE] [--a ID] [--b ID]` | two captures compared: loudness and bands per track, findings gone, new and remaining. Capture ids are listed by `mort map`; by default the latest two |
| `mort view [--open]` | the map in a browser, for the person |

### Composing

| Command | Does |
|---|---|
| `mort compose show SECTION [--part P] [--notes] [--sheet PATH]` | voiced chords (or, with `--notes`, the notes) the sheet produces; writes nothing |
| `mort compose write [--part P] [--section S] [--force]` | write the parts as MIDI, one item per part per section |

## 5. Working loops

### Judge an edit by measuring it

This is the core loop. Change one thing at a time.

1. `mort measure RANGE` if the range has no fresh capture (`mort map`).
2. `mort findings RANGE` and `mort summary RANGE`. Pick one problem.
3. Look closer: `mort report TRACK RANGE`, or a band profile:

   ```sh
   mort frames Pad 9.1..17.1 -f band:160,band:250,band:315,band:400,band:630 --step 30
   ```

   (`--step` is in seconds; a step longer than the range gives one row,
   the average.)

4. Make one edit (`mort set`, `mort fx add`, `mort automate`…). Read it
   back with `mort params`.
5. `mort measure --stale`.
6. `mort compare RANGE`. Check that the bands you aimed at moved, that
   the finding went, and that nothing else got worse.
7. Report the numbers to the person, and ask them to listen. If the edit
   did not help, `mort undo`.

### Build a session

Write the session as a script, so it is one readable file and few undo
points: tracks, FX, parameters and volumes, then sends with a `commit`
after the `send add` lines (section 9), then `send set`. Add regions for the sections, because summaries and
findings are organised by them. Keep the script in the project folder.

### Program a synth

1. `mort params TRACK FX` to see every parameter, its value and display.
2. Find a parameter's range by setting 0, 0.25, 0.5, 0.75 and 1 with
   `--normalised` and reading the display each time.
3. Set by display text where the plugin accepts it, by `--normalised`
   otherwise, and read back.
4. Measure a short range (`mort measure RANGE TRACK`) and check with
   `mort frames` that the track makes a sound at all (section 7) before
   balancing it.

### Write MIDI the person can hear

Put the notes on a track with an instrument, `mort play` the range, and
ask the person to listen. Measure as well: a track whose frames are at
−150 dB over the range has not made a sound.

## 6. Rules

- Read before you write. Know the track, FX and parameter names from
  `mort tracks` and `mort params` before editing.
- One change, then measure, then compare. Do not stack several edits and
  then measure once: you will not know which did what.
- Say what you measured, in numbers and units. Do not say you heard
  anything.
- Use musical units the person uses: bars and beats, dB, Hz, note names.
- Before measuring, check `mort tracks` for `M` and `S` flags. REAPER
  does not process muted tracks, and under a solo it renders only the
  soloed tracks (section 9). Ask the person before unmuting or
  unsoloing anything they set.
- Respect the person's work. Never remove or overwrite tracks, items or
  plugins you did not add without asking. `mort undo` only undoes mort's
  edits; do not try to undo theirs another way.
- On `stale`, re-read and reconsider; do not retry blindly.
- On `busy`, stop and tell the person.
- Prefer scripts for more than three or four edits.
- When you finish, list what you changed. Each edit's undo label starts
  with `mort:`, so the person can find it in REAPER's undo history.

## 7. Reading measurements

- **Loudness** is in LUFS (ITU-R BS.1770): momentary (400 ms) and
  short-term (3 s). Section loudness in `mort summary` is the master's.
- **Peak**: true peak in dBTP. The `headroom` finding fires above
  −1 dBTP on the master.
- **Bands**: third-octave mid-channel levels in dB, by centre frequency
  (`band:315`). `side:<Hz>` gives octave side levels.
- **Tilt**: the spectrum's slope in dB per octave; negative is darker.
- **Width**: side level relative to mid, and L/R correlation (overall,
  and below 150 Hz).
- **Centroid**: the spectral centroid in Hz, a rough measure of
  brightness.
- **Crest**: peak level minus RMS level, in dB. High crest means sharp
  transients; low crest means sustained or compressed sound.
- **Density**: onsets per second, from spectral flux. `mort summary`
  shows it as `onsets/s`.
- **Share**: a track's fraction of the energy of the bus or master it
  feeds, overall or per band.
- **Track levels**: probes sit after a track's FX and before its fader.
  `mort report` and `mort frames` show a track's own level before the
  fader. `mort summary` estimates each track's level in the mix by
  applying the fader and pan (but see section 9 for automation).
- **Silence** reads as −150 dB.
- **Findings** each carry a severity from 0 to 1, the tracks and bands
  involved, and a one-line remedy. Masking against a reverb return is
  often the blend the person wants; use judgment.

## 8. Composing from a song sheet

You write the sheet. The project must be saved, and every part's track
must already exist with an instrument on it: `mort compose write`
writes MIDI to it, and stops with a message if the track is missing.

The sheet is `<project>.mort/sheet.kdl`, next to the project file
(`song.rpp` has `song.mort/sheet.kdl`). Each section is named after a
region in the project, and its length in bars must match the region's
length through the tempo map; if not, `mort compose write` stops and
names both.

```kdl
meter "4/4"
section "verse" bars=8 {
    harmony "Dm9" "G13" bars=1          // each lasts one bar, in turn
    harmony "D dorian" bars=6           // a mode held for six bars
}
section "chorus" bars=4 {
    harmony "Gm9" "C13" "Fmaj9" "A7alt" bars=1 scale="D aeolian"
}
part "Keys" {                           // voiced first: later parts avoid it
    voicing "drop-2" voices=4 range="F3..C5"
    articulation "bossa comp"
    feel timing=6 velocity=10 seed=2
}
part "Bass" {
    voicing "root fifth" voices=2 range="A1..D3"
    articulation "bossa bass" range="E1..G3"
}
part "Pad" {
    voicing "guide tones" voices=3 range="A4..C6"
    nct "suspension" "passing" beats=1  // non-chord tones between chords
    fix "verse" bar=3 "C5 E5 A5"        // a voicing written out by hand
}
part "Drums" {
    articulation "bossa drums" kit="gm"
}
```

- Harmony is a chord symbol (`Dm7`, `G7b9`, `Cmaj7#11`, `F/A`,
  `Bbmaj7sus2`, `C7alt`) or a root and mode (`D dorian`). Each chord
  gets a default scale; `scale=` overrides it (use it to keep a
  progression diatonic).
- Voicing styles: `close`, `drop-2`, `drop-3`, `shell`, `rootless`,
  `spread`, `quartal`, `so what`, `upper structure`, `cluster`,
  `root fifth`, `guide tones`.
- Patterns, named by a part's `articulation` line: `sustain` (the
  default), `block`, `arpeggio up`,
  `arpeggio down`, `bossa piano`, `bossa comp`, `bossa bass`, `walking`,
  `rock drums`, `bossa drums`. `bossa piano` is for solo piano, with the
  bass in octaves in the left hand; use `bossa comp` when another part
  plays the bass. Drum patterns name kit pieces (`kick`, `snare`,
  `hat_closed`…), and `kit=` names the map from pieces to MIDI notes;
  `gm` is General MIDI. A project defines its own kits with `kit` nodes
  in a file in `<project>.mort/patterns/`.
- `meter` sets the time signature for sections that do not give their
  own (`section "x" bars=8 meter="6/8"`). It should match the project's;
  mort checks only that each section's length matches its region.
- `fix SECTION bar=N [beat=N] "PITCHES"` replaces the voicing of the
  harmony that starts at that bar and beat of the section (counted from
  1 within the section). It must have as many notes as the part has
  voices.
- A part with `from "Keys" voice="bass"` (in place of its `voicing`
  line) plays another part's voices.
  Voice names: `bass` (the lowest), `inner` or `inner1`, `inner2`…,
  `top`, `all`.
- `range=` on `voicing` limits the notes the voicing may use.
  `range=` on `articulation` is the instrument's playable range: notes
  outside it are reported, not moved.
- `nct` adds non-chord tones between chords, in order of preference;
  `beats=` is how long each lasts.
- `feel`: `timing` is the largest timing deviation in ticks (960 per
  quarter note), `velocity` the largest velocity deviation, `swing` from
  0.5 (straight) to 0.8, and `seed` picks a different but repeatable
  variation.
- Parts are voiced in the order the sheet lists them. Later parts avoid
  semitone clashes with earlier ones, so list the most important first.
- New voicing styles and patterns go in `<project>.mort/voicings/` and
  `<project>.mort/patterns/` as `.kdl` files. The formats are documented
  at the top of the built-in files in the mort repository,
  `crates/mort-compose/voicings/builtin.kdl` and
  `crates/mort-compose/patterns/builtin.kdl`.

The loop:

1. `mort compose show SECTION` and read the voicings and roles.
2. Adjust the sheet until they are what you want.
3. `mort compose write`. It writes only items whose notes would change
   and says why each was written. An item the person has edited is left
   alone and reported; overwrite it only if they agree (`--force`).
4. Play and measure as usual.

## 9. Known problems

These affected results when this guide was written. The numbers are
tickets in the mort repository's `tickets/open/`; the last row is a
behaviour of some synths, not a mort ticket.

| Problem | Workaround |
|---|---|
| A CLAP plugin's parameter change reads back as the old value until audio has run (0033). Undoing a later edit can also put such a parameter back to a value it had before the change. | After setting CLAP parameters, `mort play` one beat, then read back with `mort params`. After an undo, check the parameters you care about. |
| Under a solo, `mort measure` renders only the soloed tracks, reports the rest as "its probe sent no frames", and replaces good master data (0036). | Check `mort tracks` for `S` before measuring. |
| `mort summary` track levels and shares ignore volume automation: it applies the fader value at the play cursor to the whole range (0035). | Judge track balance from `mort report` and `mort frames` (pre-fader) and the master's numbers. |
| Setting a ReaEQ gain by text lands twice as far (−6 becomes −12) (0037). | Set ReaEQ gains normalised: value = 0.5 × 10^(dB/20). Read back. |
| ReaEQ, when added, has a fifth band, a high-pass, that is switched off; mort cannot turn it on (0038). | Use the low shelf at −12 dB or so as a low cut. Measure to confirm. |
| In a script, a send added in the held batch cannot be set until the batch is sent (0034). | Put `commit` after `send add`. |
| mort cannot write audio files (0039). | `docs/render-wav.lua.tmpl` in the mort repository renders WAV from inside REAPER; instructions are at its top. |
| `mort call project.save '{"path": …}'` writes a copy; the open project keeps its old path (0030). | Ask the person to use *Save as* in REAPER. |
| `register_clash` reads drum MIDI as pitches (0032). | Ignore register clashes on drum tracks. |
| Some synths need a gap between notes to restart their envelope. A held note that decays to silence can leave the next note silent. | Measure the part. If notes are missing, use a longer decay or a sustaining envelope. |
