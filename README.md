# mort

mort lets a program, or a person at a shell, work inside a running
[REAPER](https://www.reaper.fm/) session. It can build tracks and FX
chains, write MIDI and automation, play or render parts of the project,
measure what the mix sounds like, and report problems such as masking,
mud or clipping. It can then change the mix and measure again to check
that the change did what it was meant to.

It was written so that a language model with a shell can compose and mix
alongside a person, but every command is meant to be readable and usable
by hand as well. In this README, *the person* means whoever is sitting at
REAPER, as distinct from mort or a model driving it.

mort works on REAPER on macOS. It does not work with other DAWs.

## What it can do

- Read the project: tracks, folders, sends, FX and their parameters,
  regions, items, MIDI, automation and the tempo map.
- Edit it: add and remove tracks, FX and sends; set volume, pan, mute and
  solo; set any plugin parameter by the text the plugin displays
  (`-3 dB`, `2.4 kHz`); load presets; write and clear automation; write
  MIDI notes and CCs, or place a `.mid` file.
- Undo its own edits. Every edit mort makes is one undo point in REAPER,
  labelled `mort: …`, and `mort undo` never undoes the person's work.
- Play a range through the speakers, or render it faster than real time
  without writing a file. `mort measure` renders a range and stores what
  the probes measured; `mort render` on its own stores nothing.
- Measure each track, bus and the master: loudness, true peak, spectrum
  in third-octave bands, stereo width, spectral centroid and transients.
  It keeps the results as a map of the mix over time, and knows which
  parts of the map are out of date because the project changed since.
- Report findings: masking between tracks, mud, harshness, headroom,
  loudness jumps between sections, wide low end and clashing low notes.
  Each finding names the tracks and the bands involved and suggests a
  remedy. mort does not apply remedies itself.
- Compare two measurements, before and after an edit, track by track and
  band by band.
- Show the whole map in a browser while you listen.
- Compose parts from a song sheet: chords or modes per section, voiced
  with a chosen style (close, drop-2, quartal, shells and others),
  voice-led, and played through rhythm patterns. Rewriting the sheet
  regenerates only the MIDI that changes.

## How it works

mort has three parts.

- **The bridge** (`reaper_mort.dylib`) is a REAPER extension. It listens
  on a local Unix socket and carries out each request on REAPER's main
  thread. Long tasks such as playing and rendering run as jobs, so
  REAPER stays responsive.
- **The probe** (`mort-probe.clap`) is a CLAP plugin that passes audio
  through unchanged and measures it. mort places one last in the chain of
  each track it measures, and on the master.
- **`mort`** is the command-line program. It talks to the bridge over the
  socket.

A *bus* here is any track that other tracks feed, as a folder or through
sends. When mort measures a track it also measures the buses it reaches
and the master.

Measurements are stored next to the project file: `song.rpp` keeps them
in a folder called `song.mort/`. The song sheet for composing lives there
too.

## Requirements

- macOS.
- REAPER 7. mort is tested with REAPER 7.67.
- The Rust toolchain, installed with [rustup](https://rustup.rs/). The
  repository pins Rust 1.95.0 in `rust-toolchain.toml`, and rustup
  fetches it on the first build.
- git.

## Getting mort

```sh
git clone https://github.com/Vulpus-Labs/mort.git
cd mort
```

## Installing

Build everything, then copy the bridge and the probe into REAPER's
folders:

```sh
cargo build --release
./target/release/mort install
```

The first build downloads mort's dependencies, including reaper-rs from
GitHub, so it needs a network connection. The repository's
`.cargo/config.toml` makes cargo fetch them with the `git` command, so
`git` must be on your `PATH`.

`mort install` puts the bridge in
`~/Library/Application Support/REAPER/UserPlugins/reaper_mort.dylib` and
the probe in `~/Library/Audio/Plug-Ins/CLAP/mort-probe.clap`. If REAPER is
running, it says so. Quit and restart REAPER to load them; REAPER finds the
new CLAP plugin on its own at start-up. Do the same after any rebuild:
REAPER loads extensions and plugins only when it starts.

Put `mort` on your `PATH`, for example with a link:

```sh
mkdir -p ~/.local/bin
ln -sf "$PWD/target/release/mort" ~/.local/bin/mort
```

(add `~/.local/bin` to `PATH` in your shell profile if it is not there
already). Keep the link pointing into the repository's `target/release`:
`mort install` finds the bridge and probe it copies relative to where
`mort` was built.

Check that it works. Start REAPER, open or create a project, and run:

```sh
mort status
```

It prints the bridge's version, the project's name and whether REAPER is
playing. If it says `no bridge running`, REAPER has not loaded the
extension: check that `reaper_mort.dylib` is in `UserPlugins` and restart
REAPER.

## A first session

Start with an empty project in REAPER. Positions are bars and beats:
`1.1` is bar 1, beat 1, and `1.1..3.1` is the two bars from there.

```sh
mort track add Keys                      # a new track
mort fx add Keys ReaSynth                # an instrument on it
mort midi Keys 1.1..3.1 C4:1.1:2b E4:+0b:2b G4:+0b:2b   # a C major chord
mort region add verse 1.1..3.1           # name the section
mort play 1.1..3.1                       # hear it
mort measure 1.1..3.1                    # measure it
mort summary                             # loudness, width and balance by section
mort findings                            # anything wrong, worst first
mort set Keys ReaSynth Volume -12dB      # change something
mort measure --stale                     # re-measure only what the edit affected
mort compare                             # what the edit changed
mort undo                                # take it back
```

The MIDI line writes three notes, each `PITCH:START:LENGTH`: C4 at bar 1
beat 1 for two beats, then E4 and G4 at `+0b`, no time after the previous
note's start, so the three sound together as a chord. Plugin parameters
take the text the plugin displays, with or without a space (`-12dB` or
`-12 dB`).

Every command has `--help`. Most commands end their output with
`state N`, the project's change counter (see *Edits and the person*
below). `--json` prints the raw result instead of text, including the
ids of tracks, FX and items.

## Main ideas

**Addressing.** Tracks can be named by number, name, part of a name, id
(as `--json` output gives it), or `master`. FX by position in the chain, name, part of a name, or id.
Parameters by number or name. When a name is ambiguous, mort lists the
matches and asks for a number.

**Positions and ranges.** `9` or `9.1` is bar 9, beat 1; `9.2.240` adds
240 ticks at 960 per quarter note; `12.5s` is seconds; `+4b` is four
quarter notes after the position before it in the same command. A range is two positions
joined by `..`, as in `9.1..17.1` or `9.1..+8b`. Positions follow the
project's tempo map and time signatures.

**Edits and the person.** mort and the person share the project. Every
edit is one labelled undo point. Each read returns the project's state
counter, and an edit can say which state it expects: if the person
changed the project in between, the edit fails with `stale` instead of
acting on old information. The `mort` command does this by itself: each
command reads the state and sends it with its edit, and a script carries
it from step to step. In REAPER's action list, *mort: hold edits*
makes the bridge refuse all edits until it is turned off again. Reads
and measurements carry on.

**Measuring and the map.** `mort measure RANGE` renders the range (or,
with `--play`, plays it) and stores a *capture* of every track, bus and
the master. Captures build up a map of the whole mix. When a track
changes, the map marks that track, and the buses it feeds, as stale for
the ranges it covered. `mort measure --stale` re-measures only those.
`mort measure` puts probes on the tracks it needs (`mort probes ensure`
puts them everywhere), and they stay. When you play the project yourself
with probes in place, mort also stores what passes through them.

**Findings.** `mort findings` judges the map against thresholds. The
defaults were set on test mixes. Each project can override them with
`mort thresholds set`, and `mort findings --all` shows how close each
measurement is to its threshold.

**Comparing.** `mort compare` sets the latest capture against the one
before it, section by section and track by track: loudness, the bands
that moved most, and the findings that appeared or went.

**The map view.** `mort view --open` serves a page on `127.0.0.1` showing
every track over time: how recently each part was measured, what is
stale, and where the findings are. It updates while you play.

**Scripts.** `mort run file.kdl` runs a list of mort commands. Edits in a
row are sent as one batch and make one undo point. A script looks like
this:

```kdl
track add Bass
fx add Bass ReaSynth
set Bass ReaSynth Volume "-12dB"
track set Bass volume=-3
measure "1.1..9.1" stale=#true
```

Each line is a mort command. An option `--volume -3` is written
`volume=-3`, and a flag such as `--stale` is written `stale=#true`. Values
that are not plain words or numbers, like `"-12dB"` and ranges, go in
quotes. While a script runs, its edits are held back and sent to REAPER
together, as one undo point, when the script reaches a step that needs
them done (`measure`, `play`, `undo` and the like), a `commit` line, or
its end.

**Composing.** `mort compose` reads a song sheet,
`<project>.mort/sheet.kdl`, that lists sections (matching the project's
regions by name), their chords or modes, and parts. Each part has a
voicing style, a range and a rhythm pattern (its `articulation`).
`mort compose show SECTION` prints the voiced chords with each note's role
in the chord (root, 3rd, 7th, 9th and so on), and `mort compose write`
writes one MIDI item per part per section. Writing again rewrites only
items whose notes would change, and leaves alone any item you have
edited by hand unless you pass `--force`.

You write the sheet by hand (or have a model write it). Before
`mort compose write`, the project must be saved, each section needs a
region of the same name and length, and each part's track must exist
with an instrument on it; mort writes MIDI to the track, and stops with
a message if the track is missing. The sheet format is described
in [adrs/0013-composition-model.md](adrs/0013-composition-model.md), and
the built-in voicing styles and patterns, with their formats, in
[crates/mort-compose/voicings/builtin.kdl](crates/mort-compose/voicings/builtin.kdl)
and
[crates/mort-compose/patterns/builtin.kdl](crates/mort-compose/patterns/builtin.kdl).
Two example sheets are in [docs/e004-sheet.kdl](docs/e004-sheet.kdl) (a
short bossa nova arrangement) and
[docs/ambient-sheet.kdl](docs/ambient-sheet.kdl) (an ambient piece).

## Using mort with an LLM

mort's commands are designed for a model working through a shell. They
take names rather than ids, print short aligned text, and every error
names what it is about and what to do next. To equip a model:

1. Give it a shell where `mort` is on the `PATH`, with REAPER running the
   bridge on the same machine. `mort status` should work from that shell.
2. Give it [docs/llm-guide.md](docs/llm-guide.md). The guide is written
   for a model to read: the command syntax, a reference for every
   command, the loops that work well, the rules to follow, and the known
   problems with their workarounds. Either paste it into the model's
   instructions, or tell the model to read it before its first mort
   command. With Claude Code, for example, add a line to the project's
   `CLAUDE.md`:

   ```text
   Before using the `mort` command, read /path/to/mort/docs/llm-guide.md.
   ```

3. Tell it what you want in musical terms ("the chorus feels muddy",
   "write a bass part that follows the chords"). The guide tells it how
   to get from there to commands.

Some practical points for working this way:

- A model cannot hear. It works from measurements, and should say what it
  measured, not what it "hears". Listen to its changes yourself.
- Put *mort: hold edits* on a toolbar button (right-click a toolbar in
  REAPER and choose *Customize toolbar*). Turning it on stops all edits
  at once, without stopping the model's reads or measurements.
- `mort undo` reverses mort's edits one at a time and never touches
  yours.
- If you solo or mute tracks while listening, tell the model, or unmute
  and unsolo them before it measures. REAPER does not process muted tracks, and
  under a solo it renders only the soloed ones.

## Limitations

- macOS and REAPER only.
- mort cannot write audio files: `mort render` measures and discards. A
  Lua script for REAPER that renders to WAV, with instructions at its
  top, is in [docs/render-wav.lua.tmpl](docs/render-wav.lua.tmpl).
- mort cannot change the tempo or time signatures, move or rename
  regions, or copy, move and split items.
- mort cannot save the project under a new name. To work in a new file,
  use *Save as* in REAPER.
- Known problems that affect results are listed, with workarounds, in
  [docs/llm-guide.md](docs/llm-guide.md#known-problems), and tracked in
  [tickets/open/](tickets/open/).

## Development

```text
crates/
  mort-proto     RPC types, ids, positions
  mort-session   the Session trait, and a fake REAPER session for tests
  mort-bridge    the REAPER extension
  mort-probe     the CLAP measuring plugin
  mort-link      shared memory between probe and bridge
  mort-features  loudness, spectrum and other features
  mort-map       captures, staleness, findings, queries
  mort-compose   song sheets, harmony, voicing, articulation
  mort-cli       the mort program
```

Most of mort runs without REAPER: the tests use a fake session that
renders through the real feature code. Before committing:

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

`scripts/smoke.sh` checks the bridge against a running REAPER; its header
explains how to start REAPER for it.

Design decisions are recorded in [adrs/](adrs/), and work in
[epics/](epics/) and [tickets/](tickets/).

## Licence

mort is licensed under either the MIT licence or the Apache License 2.0,
at your option, as stated in `Cargo.toml`.
