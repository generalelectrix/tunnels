# audio_vis

Standalone audio visualization tool for developing and tuning the audio processing pipeline. Not part of the production system — this is a dev/experimentation tool.

## How it works

Creates a `Processor` directly (no show loop) and plots envelope traces via `egui_plot`. Controls write directly to `ProcessorSettings` atomics. This is acceptable because there's no show loop to mediate — the production GUI uses message-passing instead.

## Snapshot tests

Signal processing snapshot tests feed shaped audio (sine bursts, impulses, repeated kicks, quiet-to-loud transitions) through the real `Processor` in ~1ms buffer chunks and plot the results. These verify that the DSP chain produces expected output shapes.

Update snapshots with: `UPDATE_SNAPSHOTS=1 cargo test -p audio_vis`

## History

This tool was the primary instrument for an extensive A/B comparison of envelope extraction approaches. A branch preserving the full comparison matrix (abs vs Hilbert × envelope follower vs two-stage vs RMS vs median, plus TKEO and three-stage variants) exists as a reference.

The winner was: Hilbert transform + two-stage (fast→slow) envelope follower + symmetric output smoother.

Research and experimental findings are documented in:
- `/Users/macklin/src/tunnels-better-audio.md` (plan)
- `/Users/macklin/src/tunnels-better-audio-raw-research.md` (raw research compilation)
