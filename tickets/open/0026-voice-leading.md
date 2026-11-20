---
id: "0026"
title: "Voice-leading: Viterbi over candidates; labelled blocks; non-chord tones"
priority: high
created: 2026-11-20
epic: E004
depends: ["0025"]
---

## Summary

Choose one voicing per harmony change, at the least total cost, and
produce labelled blocks (ADR 0013 §6, §7).

## Acceptance criteria

- [ ] Viterbi over the candidates. Hard constraints: ranges, voice
      count, a required top note, and literal voicings given in the
      sheet. Soft costs, weighted per part: movement, leaps, common tones,
      low-interval spacing, parallel fifths and octaves, crossing.
- [ ] Deterministic tie-breaking. The same input gives the same blocks,
      tested.
- [ ] When the hard constraints cannot all be met, the error names the
      section, bar and constraint, and suggests what to relax.
- [ ] Blocks: start, length, and notes labelled with voice (`bass`,
      `inner1…`, `top`) and role.
- [ ] Optional non-chord tones between blocks: passing, neighbour,
      suspension, anticipation and pedal, each labelled.
- [ ] Sheet syntax for a part's voicing spec, cost weights, literal
      voicings and top-note constraints.
- [ ] Decide whether walking bass is a line generator here or an
      articulation (ADR 0013, Consequences), and record the choice in the
      ADR.
- [ ] Tests: a ii–V–I keeps common tones; a forced top line is honoured;
      an impossible range fails with a clear error; passing tones fill
      thirds when switched on.
