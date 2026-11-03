---
id: adr-wal-v3-fixes
kind: decision
title: Run the prefix cut inside open after recovery and tail repair, and refuse inconsistent segment and log pairs
status: confirmed
date: 2026-10-29
parent: proj-atlas
caused_by: obs-wal-v3-review-defect
derived_from: adr-segments-checkpoint
---
The cut now runs after recovery-marker handling and tail repair. The engine refuses a log whose base is past the newest segment, and a segment past the log end. Six new tests cover marker plus segment, torn tail plus segment and missing segment. Independent re-review of the fixed digest is still outstanding.
