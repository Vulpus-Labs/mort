# The E001 proof

Ticket 0015, run on 2026-11-19 in REAPER 7.67 on macOS. The bridge was
built from commit `efe0cab` with the 0015 changes on top (region support,
and the rollback fix described below); the probe and bridge were release
builds. A new, empty project was driven with `mort` commands alone, except
for the two steps the epic asks a person to do by hand. Every command and
its full output is below, as run.

The script used for the fix is [e001-fix.kdl](e001-fix.kdl).

## 1. Build the session

Four tracks, three of them silkscreen instruments (vxn-1c, vxn-2b and
Poly-Kritter) and ReaSynth; two regions; MIDI on every track; volume
automation on Keys. Two problems are planted:

- **Masking:** Pad sustains E2, G2 and B2, inside Bass's range.
- **Mud:** Warm stacks C3–D4 in the low mids.

```text
$ mort status
bridge   0.1.0+efe0cab1bc
project  untitled
held     no
playing  no
jobs     none running, 0 queued
state    1

$ mort tracks
#  name    volume  pan    fx / sends
   MASTER  0.0 dB  C

flags: M muted, S soloed, x not sent to its parent; (fx) bypassed
state 1

$ mort track add Bass
added Bass  {4C9E3D20-1ED2-CA47-BA30-1AD648EB96A8}
state 2

$ mort track add Pad
added Pad  {228E4DBB-E142-9045-BD0A-7D2D18CC0543}
state 3

$ mort track add Warm
added Warm  {B1DD683F-D1FD-6940-8184-BBBE71FAE80D}
state 4

$ mort track add Keys
added Keys  {1F7A15AD-121D-B940-8C55-B085601413F9}
state 5

$ mort fx add Bass vxn-1c
added CLAPi: vxn-1c (Vulpus Labs) to Bass at 1  {8005EDD5-3E1C-204B-948C-4F7DD621E39A}
state 6

$ mort fx add Pad vxn-2b
added CLAPi: vxn-2b (Vulpus Labs) to Pad at 1  {A70264D9-1245-9A4A-A989-9F1759334912}
state 7

$ mort fx add Warm Poly-Kritter
mort: not_found: no such plugin (Poly-Kritter). use the name REAPER's FX browser shows, such as `ReaEQ` or `CLAP: mort probe`
(exit 1)

$ mort fx add Keys ReaSynth
added VSTi: ReaSynth (Cockos) to Keys at 1  {5D417674-E8CF-F642-AC00-9912AA48AC57}
state 9

$ mort fx add Warm PolyKritter
mort: not_found: no such plugin (PolyKritter). use the name REAPER's FX browser shows, such as `ReaEQ` or `CLAP: mort probe`
(exit 1)

$ mort fx add Warm "silkscreen Poly-Kritter"
added CLAPi: silkscreen Poly-Kritter (Vulpus Labs) to Warm at 1  {59173BA3-0B1C-614E-8D1D-6FEB1AE566C6}
state 11

$ mort tracks
#  name    volume  pan    fx / sends
1  Bass    0.0 dB  C      CLAPi: vxn-1c (Vulpus Labs)
2  Pad     0.0 dB  C      CLAPi: vxn-2b (Vulpus Labs)
3  Warm    0.0 dB  C      CLAPi: silkscreen Poly-Kritter (Vulpus Labs)
4  Keys    0.0 dB  C      VSTi: ReaSynth (Cockos)
   MASTER  0.0 dB  C

flags: M muted, S soloed, x not sent to its parent; (fx) bypassed
state 11

$ mort region add verse 1.1..5.1
added region 1 `verse` at 0.00–8.00s
state 12

$ mort region add chorus 5.1..9.1
added region 2 `chorus` at 8.00–16.00s
state 13

$ mort midi Bass 1.1..9.1 E1:1.1:8b C2:3.1:8b A1:5.1:8b B1:7.1:8b
Bass: wrote 4 notes, 0 CCs into a new item
Bass: item 1.1–9.1, 4 notes, 0 CCs
  1.1–3.1  E1 (28)  v100  ch1
  3.1–5.1  C2 (36)  v100  ch1
  5.1–7.1  A1 (33)  v100  ch1
  7.1–9.1  B1 (35)  v100  ch1
state 14

$ mort midi Pad 1.1..9.1 E2:1.1:32b G2:+0b:32b B2:+0b:32b
Pad: wrote 3 notes, 0 CCs into a new item
Pad: item 1.1–9.1, 3 notes, 0 CCs
  1.1–9.1  E2 (40)  v100  ch1
  1.1–9.1  G2 (43)  v100  ch1
  1.1–9.1  B2 (47)  v100  ch1
state 15

$ mort midi Warm 1.1..9.1 C3:1.1:32b:90 E3:+0b:32b:90 G3:+0b:32b:90 B3:+0b:32b:90 D4:+0b:32b:90
Warm: wrote 5 notes, 0 CCs into a new item
Warm: item 1.1–9.1, 5 notes, 0 CCs
  1.1–9.1  C3 (48)  v90  ch1
  1.1–9.1  E3 (52)  v90  ch1
  1.1–9.1  G3 (55)  v90  ch1
  1.1–9.1  B3 (59)  v90  ch1
  1.1–9.1  D4 (62)  v90  ch1
state 16

$ mort midi Keys 1.1..9.1 C5:1.1:0.5b:80 E5:+1b:0.5b:80 G5:+1b:0.5b:80 B5:+1b:0.5b:80 C5:+1b:0.5b:80 E5:+1b:0.5b:80 G5:+1b:0.5b:80 B5:+1b:0.5b:80 C5:+1b:0.5b:80 E5:+1b:0.5b:80 G5:+1b:0.5b:80 B5:+1b:0.5b:80 C5:+1b:0.5b:80 E5:+1b:0.5b:80 G5:+1b:0.5b:80 B5:+1b:0.5b:80 C5:+1b:0.5b:80 E5:+1b:0.5b:80 G5:+1b:0.5b:80 B5:+1b:0.5b:80 C5:+1b:0.5b:80 E5:+1b:0.5b:80 G5:+1b:0.5b:80 B5:+1b:0.5b:80 C5:+1b:0.5b:80 E5:+1b:0.5b:80 G5:+1b:0.5b:80 B5:+1b:0.5b:80 C5:+1b:0.5b:80 E5:+1b:0.5b:80 G5:+1b:0.5b:80 B5:+1b:0.5b:80
Keys: wrote 32 notes, 0 CCs into a new item
Keys: item 1.1–9.1, 32 notes, 0 CCs
  1.1–1.1.480  C5 (72)  v80  ch1
  1.2–1.2.480  E5 (76)  v80  ch1
  1.3–1.3.480  G5 (79)  v80  ch1
  1.4–1.4.480  B5 (83)  v80  ch1
  2.1–2.1.480  C5 (72)  v80  ch1
  2.2–2.2.480  E5 (76)  v80  ch1
  2.3–2.3.480  G5 (79)  v80  ch1
  2.4–2.4.480  B5 (83)  v80  ch1
  3.1–3.1.480  C5 (72)  v80  ch1
  3.2–3.2.480  E5 (76)  v80  ch1
  3.3–3.3.480  G5 (79)  v80  ch1
  3.4–3.4.480  B5 (83)  v80  ch1
  4.1–4.1.480  C5 (72)  v80  ch1
  4.2–4.2.480  E5 (76)  v80  ch1
  4.3–4.3.480  G5 (79)  v80  ch1
  4.4–4.4.480  B5 (83)  v80  ch1
  5.1–5.1.480  C5 (72)  v80  ch1
  5.2–5.2.480  E5 (76)  v80  ch1
  5.3–5.3.480  G5 (79)  v80  ch1
  5.4–5.4.480  B5 (83)  v80  ch1
  6.1–6.1.480  C5 (72)  v80  ch1
  6.2–6.2.480  E5 (76)  v80  ch1
  6.3–6.3.480  G5 (79)  v80  ch1
  6.4–6.4.480  B5 (83)  v80  ch1
  7.1–7.1.480  C5 (72)  v80  ch1
  7.2–7.2.480  E5 (76)  v80  ch1
  7.3–7.3.480  G5 (79)  v80  ch1
  7.4–7.4.480  B5 (83)  v80  ch1
  8.1–8.1.480  C5 (72)  v80  ch1
  8.2–8.2.480  E5 (76)  v80  ch1
  8.3–8.3.480  G5 (79)  v80  ch1
  8.4–8.4.480  B5 (83)  v80  ch1
state 17

$ mort automate Keys volume 1.1=-6 5.1=0
Keys volume: wrote 2 points
Keys volume: 2 points
  1.1  0.000s  -6.00dB  linear
  5.1  8.000s  0.00dB   linear
state 19

$ mort tracks
#  name    volume  pan    fx / sends
1  Bass    0.0 dB  C      CLAPi: vxn-1c (Vulpus Labs)
2  Pad     0.0 dB  C      CLAPi: vxn-2b (Vulpus Labs)
3  Warm    0.0 dB  C      CLAPi: silkscreen Poly-Kritter (Vulpus Labs)
4  Keys    0.0 dB  C      VSTi: ReaSynth (Cockos)
   MASTER  0.0 dB  C

flags: M muted, S soloed, x not sent to its parent; (fx) bypassed
state 19
```

## 2. Measure

The first measurement found the session far too hot (+6.3 dBTP on the
master) and Bass 6.5 dB under Pad, just outside the masking spread. It
also found a problem nobody planted: Pad's B2 against Warm's C3 is a minor
second.

```text
$ mort measure 1.1..9.1

measured 0.00–16.00s by render in 1.1 s: capture 1
  track   frames
  MASTER  320
  Bass    320
  Pad     320
  Warm    320
  Keys    320
in memory only: save the project to keep the map
state 20

$ mort summary
verse 0.00–8.00s  -5.2 LUFS, peak 6.3 dBTP, tilt -2.6 dB/oct, side -60 dB, 0.6 onsets/s
  findings: headroom 1, register_clash 1
  track  level  share  centroid  loudest
  Warm   -7.4   52%    603 Hz    315 Hz
  Pad    -11.3  30%    99 Hz     80 Hz
  Keys   -13.4  13%    597 Hz    800 Hz
  Bass   -17.8  10%    99 Hz     63 Hz
chorus 8.00–16.00s  -6.1 LUFS, peak 4.1 dBTP, tilt -1.8 dB/oct, side -60 dB, 0.0 onsets/s
  findings: headroom 1, mud 1, register_clash 1
  track  level  share  centroid  loudest
  Warm   -7.3   70%    604 Hz    250 Hz
  Keys   -13.4  17%    597 Hz    800 Hz
  Bass   -17.4  13%    104 Hz    63 Hz
  Pad    -27.9  1%     98 Hz     80 Hz
level: LUFS in the mix (fader and pan applied); share: of the master's energy
state 20

$ mort findings
verse 0.00–8.00s
  1.00 headroom       the master peaks at 6.3 dBTP at 0.20s: lower the mix into the master, or limit it to -1 dBTP
  1.00 register_clash Pad (B2) and Warm (C3) sustain a minor second apart low down, first at 0.00s: revoice one, or move it an octave
chorus 8.00–16.00s
  1.00 headroom       the master peaks at 4.1 dBTP at 9.00s: lower the mix into the master, or limit it to -1 dBTP
  1.00 register_clash Pad (B2) and Warm (C3) sustain a minor second apart low down, first at 8.00s: revoice one, or move it an octave
  0.40 mud            MASTER has 4.4 dB more 200–500 Hz than its tilt predicts: cut 200–500 Hz on Warm
5 findings
state 20
```

## 3. Set levels, re-measure what is stale

Fader moves make each track and the master stale; `--stale` re-measures
them. Both planted problems are now reported: Bass and Pad compete at
80–125 Hz in the verse, and the master has 2.9 dB more 200–500 Hz than its
tilt predicts in the chorus, with Warm named. Bass's harmonics also
compete with Warm at 125–500 Hz.

```text
$ mort track set Bass --volume -6
set Bass volume -6.0 dB
state 21

$ mort track set Pad --volume -14
set Pad volume -14.0 dB
state 22

$ mort track set Warm --volume -12
set Warm volume -12.0 dB
state 23

$ mort track set Keys --volume -10
set Keys volume -10.0 dB
state 24

$ mort map
1 captures
  #  range        mode    taken    tracks
  1  0.00–16.00s  render  14s ago  5
  track   covered      stale
  MASTER  0.00–16.00s  0.00–16.00s (Keys changed)
  Bass    0.00–16.00s  0.00–16.00s (Bass changed)
  Pad     0.00–16.00s  0.00–16.00s (Pad changed)
  Warm    0.00–16.00s  0.00–16.00s (Warm changed)
  Keys    0.00–16.00s  0.00–16.00s (Keys changed)
5 tracks stale: re-measure with `mort measure --stale`
in memory only: save the project to keep the map
state 24

$ mort measure --stale

measured 0.00–16.00s by render in 0.7 s: capture 2
  track   frames
  MASTER  320
  Bass    320
  Pad     320
  Warm    320
  Keys    320
superseded captures 1 (kept for `mort compare`)
in memory only: save the project to keep the map
state 25

$ mort findings
verse 0.00–8.00s
  1.00 register_clash Pad (B2) and Warm (C3) sustain a minor second apart low down, first at 0.00s: revoice one, or move it an octave
  0.89 masking        Bass and Warm compete at 125–500 Hz in MASTER: cut one there, or move one in register or pan
  0.57 masking        Bass and Pad compete at 80–125 Hz in MASTER: cut one there, or move one in register or pan
chorus 8.00–16.00s
  1.00 register_clash Pad (B2) and Warm (C3) sustain a minor second apart low down, first at 8.00s: revoice one, or move it an octave
  0.60 masking        Bass and Warm compete at 125–500 Hz in MASTER: cut one there, or move one in register or pan
  0.15 mud            MASTER has 2.9 dB more 200–500 Hz than its tilt predicts: cut 200–500 Hz on Warm
6 findings
state 25

$ mort summary
verse 0.00–8.00s  -16.7 LUFS, peak -5.6 dBTP, tilt -2.7 dB/oct, side -60 dB, 0.5 onsets/s
  findings: masking 2, register_clash 1
  track  level  share  centroid  loudest
  Warm   -19.4  43%    603 Hz    315 Hz
  Keys   -23.4  17%    597 Hz    800 Hz
  Bass   -23.8  32%    99 Hz     63 Hz
  Pad    -25.3  16%    99 Hz     80 Hz
chorus 8.00–16.00s  -16.9 LUFS, peak -6.7 dBTP, tilt -2.3 dB/oct, side -60 dB, 0.1 onsets/s
  findings: masking 1, mud 1, register_clash 1
  track  level  share  centroid  loudest
  Warm   -19.3  48%    604 Hz    250 Hz
  Bass   -23.4  36%    104 Hz    63 Hz
  Keys   -23.4  18%    597 Hz    800 Hz
  Pad    -41.9  0%     98 Hz     80 Hz
level: LUFS in the mix (fader and pan applied); share: of the master's energy
state 25
```

## 4. Fix with ReaEQ through a script

The script adds ReaEQ to Pad (a 24 dB low-shelf cut at 200 Hz) and to Warm
(a 9 dB cut at 300 Hz). Its six edits are held and sent as one batch, with
one undo point, when `measure` needs them made. `compare` shows both
planted findings gone: Pad fell about 30 dB below 125 Hz, and Warm about
14 dB at 250–400 Hz. The Bass/Warm masking stayed, a little broader,
because Warm's cut brought it level with Bass's harmonics.

```text
$ mort run docs/e001-fix.kdl

> fx add Pad ReaEQ
  held for the batch
> set Pad ReaEQ "Freq-Low Shelf" 200
  held for the batch
> set Pad ReaEQ "Gain-Low Shelf" -24
  held for the batch
> fx add Warm ReaEQ
  held for the batch
> set Warm ReaEQ "Freq-Band 2" 300
  held for the batch
> set Warm ReaEQ "Gain-Band 2" -9
  held for the batch
> set Warm ReaEQ "BW-Band 2" 2.5
  held for the batch
> measure 1.1..9.1
  measured 0.00–16.00s by render in 0.7 s: capture 3
    track   frames
    MASTER  320
    Bass    320
    Pad     320
    Warm    320
    Keys    320
  superseded captures 2 (kept for `mort compare`)
  in memory only: save the project to keep the map
> findings
  verse 0.00–8.00s
    1.00 register_clash Pad (B2) and Warm (C3) sustain a minor second apart low down, first at 0.00s: revoice one, or move it an octave
    0.99 masking        Bass and Warm compete at 125–800 Hz in MASTER: cut one there, or move one in register or pan
  chorus 8.00–16.00s
    1.00 masking        Bass and Warm compete at 125–800 Hz in MASTER: cut one there, or move one in register or pan
    1.00 register_clash Pad (B2) and Warm (C3) sustain a minor second apart low down, first at 8.00s: revoice one, or move it an octave
  4 findings
> compare
  capture 2 → 3 over 0.00–16.00s
  verse 0.00–8.00s
    track   loudness       bands that moved most
    Pad     -11.3 → -37.6  50 Hz -31.0, 63 Hz -31.0, 40 Hz -30.8
    Warm    -7.4 → -14.0   315 Hz -15.0, 250 Hz -14.2, 400 Hz -13.5
    MASTER  -16.7 → -20.4  100 Hz -9.9, 160 Hz -9.2, 315 Hz -9.0
    2 other tracks moved less than 0.5 dB
  chorus 8.00–16.00s
    track   loudness       bands that moved most
    Pad     -27.9 → -54.6  80 Hz -30.0, 100 Hz -28.1, 125 Hz -24.3
    Warm    -7.3 → -13.9   315 Hz -15.0, 250 Hz -14.2, 400 Hz -13.5
    MASTER  -16.9 → -19.4  315 Hz -9.5, 250 Hz -6.6, 200 Hz -6.4
    2 other tracks moved less than 0.5 dB
  findings gone:
    masking        Bass and Pad compete at 80–125 Hz in MASTER: cut one there, or move one in register or pan (0.57)
    mud            MASTER has 2.9 dB more 200–500 Hz than its tilt predicts: cut 200–500 Hz on Warm (0.15)
  findings still there:
    register_clash Pad (B2) and Warm (C3) sustain a minor second apart low down, first at 0.00s: revoice one, or move it an octave (1.00 → 1.00)
    masking        Bass and Warm compete at 125–800 Hz in MASTER: cut one there, or move one in register or pan (0.89 → 0.99)
    register_clash Pad (B2) and Warm (C3) sustain a minor second apart low down, first at 8.00s: revoice one, or move it an octave (1.00 → 1.00)
    masking        Bass and Warm compete at 125–800 Hz in MASTER: cut one there, or move one in register or pan (0.60 → 1.00)
state 27
```

## 5. A ReaEQ edit made by hand

By hand in REAPER, the person raised Band 3 of Warm's ReaEQ to +11.4 dB.
`mort map` shows Warm and the master (the bus it feeds) stale, and nothing
else.

```text
$ mort map
1 captures
  #  range        mode    taken   tracks
  3  0.00–16.00s  render  9m ago  5
  track   covered      stale
  MASTER  0.00–16.00s  0.00–16.00s (Warm changed)
  Bass    0.00–16.00s  -
  Pad     0.00–16.00s  -
  Warm    0.00–16.00s  0.00–16.00s (Warm changed)
  Keys    0.00–16.00s  -
2 tracks stale: re-measure with `mort measure --stale`
in memory only: save the project to keep the map
state 28

$ mort params Warm ReaEQ Gain
VST: ReaEQ (Cockos) on Warm: 6 of 19 parameters
  #   name               value  display
  1   Gain-Low Shelf     0.500  0.0
  4   Gain-Band 2        0.089  -15.0
  7   Gain-Band 3        0.953  11.4
  10  Gain-High Shelf 4  0.500  0.0
  13  Gain-High Pass 5   0.500  0.0
  15  Global Gain        0.500  0.0
state 28

$ mort measure --stale

measured 0.00–16.00s by render in 0.7 s: capture 4
  track   frames
  MASTER  320
  Warm    320
in memory only: save the project to keep the map
state 29

$ mort findings --kind masking,mud
verse 0.00–8.00s
  0.99 masking        Bass and Warm compete at 125–500 Hz in MASTER: cut one there, or move one in register or pan
chorus 8.00–16.00s
  1.00 masking        Bass and Warm compete at 125–500 Hz in MASTER: cut one there, or move one in register or pan
2 findings
state 29
```

## 6. Undo mort's fix from REAPER's undo history

In REAPER's undo history, the person went back to the point before
`mort: script e001-fix.kdl`, which also undid the hand edit. `mort map`
shows Pad, Warm and the master stale. `measure --stale` re-measures just
those three, and both findings return with the same values as before the
fix. `compare` lists them as new.

```text
$ mort tracks
#  name    volume    pan    fx / sends
1  Bass    -6.0 dB   C      CLAPi: vxn-1c (Vulpus Labs), CLAP: mort probe (Vulpus Labs)
2  Pad     -14.0 dB  C      CLAPi: vxn-2b (Vulpus Labs), CLAP: mort probe (Vulpus Labs)
3  Warm    -12.0 dB  C      CLAPi: silkscreen Poly-Kritter (Vulpus Labs), CLAP: mort probe (Vulpus Labs)
4  Keys    -10.0 dB  C      VSTi: ReaSynth (Cockos), CLAP: mort probe (Vulpus Labs)
   MASTER  0.0 dB    C      CLAP: mort probe (Vulpus Labs)

flags: M muted, S soloed, x not sent to its parent; (fx) bypassed
state 30

$ mort map
2 captures
  #  range        mode    taken    tracks
  3  0.00–16.00s  render  10m ago  3
  4  0.00–16.00s  render  1m ago   2
  track   covered      stale
  MASTER  0.00–16.00s  0.00–16.00s (Pad changed)
  Bass    0.00–16.00s  -
  Pad     0.00–16.00s  0.00–16.00s (Pad changed)
  Warm    0.00–16.00s  0.00–16.00s (Warm changed)
  Keys    0.00–16.00s  -
3 tracks stale: re-measure with `mort measure --stale`
in memory only: save the project to keep the map
state 30

$ mort measure --stale

measured 0.00–16.00s by render in 0.6 s: capture 5
  track   frames
  MASTER  320
  Pad     320
  Warm    320
superseded captures 4 (kept for `mort compare`)
in memory only: save the project to keep the map
state 31

$ mort findings --kind masking,mud
verse 0.00–8.00s
  0.89 masking        Bass and Warm compete at 125–500 Hz in MASTER: cut one there, or move one in register or pan
  0.57 masking        Bass and Pad compete at 80–125 Hz in MASTER: cut one there, or move one in register or pan
chorus 8.00–16.00s
  0.60 masking        Bass and Warm compete at 125–500 Hz in MASTER: cut one there, or move one in register or pan
  0.15 mud            MASTER has 2.9 dB more 200–500 Hz than its tilt predicts: cut 200–500 Hz on Warm
4 findings
state 31

$ mort compare
capture 4 → 5 over 0.00–16.00s
verse 0.00–8.00s
  track   loudness       bands that moved most
  Pad     -37.6 → -11.3  50 Hz +31.0, 63 Hz +31.0, 40 Hz +30.8
  Warm    -8.5 → -7.4    250 Hz +12.2, 315 Hz +12.1, 200 Hz +10.6
  MASTER  -18.0 → -16.7  100 Hz +9.9, 315 Hz +8.7, 160 Hz +8.6
  2 other tracks moved less than 0.5 dB
chorus 8.00–16.00s
  track   loudness       bands that moved most
  Pad     -54.6 → -27.9  80 Hz +30.0, 100 Hz +28.1, 125 Hz +24.3
  Warm    -8.4 → -7.3    250 Hz +12.2, 315 Hz +12.1, 200 Hz +10.6
  MASTER  -17.4 → -16.9  315 Hz +8.7, 250 Hz +6.5, 1.2 kHz -6.2
  2 other tracks moved less than 0.5 dB
findings new:
  masking        Bass and Pad compete at 80–125 Hz in MASTER: cut one there, or move one in register or pan (0.57)
  mud            MASTER has 2.9 dB more 200–500 Hz than its tilt predicts: cut 200–500 Hz on Warm (0.15)
findings still there:
  register_clash Pad (B2) and Warm (C3) sustain a minor second apart low down, first at 0.00s: revoice one, or move it an octave (1.00 → 1.00)
  masking        Bass and Warm compete at 125–500 Hz in MASTER: cut one there, or move one in register or pan (0.99 → 0.89)
  masking        Bass and Warm compete at 125–500 Hz in MASTER: cut one there, or move one in register or pan (1.00 → 0.60)
  register_clash Pad (B2) and Warm (C3) sustain a minor second apart low down, first at 8.00s: revoice one, or move it an octave (1.00 → 1.00)
state 31
```

## Found on the way

- **Failed edits were never rolled back in REAPER.** It does not move the
  project state count when an undo block opens or inside it: it moves it
  by one when the block closes, even an empty one. The dispatcher judged
  "did this edit change anything" by the count inside the block, so a
  failed edit's partial changes stayed. It now judges by whether the top
  undo label became the edit's own. When two edits in a row share a label,
  the second is numbered (`mort: add bass (2)`), so they can be told
  apart. The fake now moves the count as REAPER does, and
  `scripts/smoke.sh` checks rollback.
- **There was no way to add a region,** which the proof needs: `mort
  region add <name> <range|position>` (RPC `region.add`) now does it.
- **REAPER's `TrackFX_AddByName` did not match "Poly-Kritter"** for
  "silkscreen Poly-Kritter (Vulpus Labs)". The full name worked. mort's
  error said to use the name the FX browser shows, which was the right
  advice.
