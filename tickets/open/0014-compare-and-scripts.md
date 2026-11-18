---
id: "0014"
title: "compare, and mort run scripts"
priority: high
created: 2026-11-18
epic: E001
depends: ["0013", "0007"]
---

## Summary

Judge edits by comparing captures. Run sequences of commands as scripts
(ADR 0009 §4).

## Acceptance criteria

- [ ] `compare(a, b, range)`: changes in features per track and section,
      and findings that appeared or disappeared. The CLI defaults to the
      latest two captures that cover the range.
- [ ] `mort run file.kdl`: edits become one `batch` carrying
      `expect_state`. A failure stops the script and names the step.
- [ ] Tested over the fake: plant masking, measure, apply an EQ cut by
      script, re-measure, compare shows the finding gone.
