# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

Shifter is a macOS TUI audio time-shift (DVR) tool in Rust. It captures audio from a BlackHole virtual audio device, buffers it in a lock-free ring buffer, and outputs to physical speakers with user-controllable pause/seek/rewind.

## Build & Test Commands

```bash
cargo build --release        # build
cargo test                   # run all tests
cargo test ring_buffer       # run ring buffer tests only
cargo run --release          # run (requires BlackHole installed)
cargo run --release -- -l    # list audio devices
```

## Architecture

Three threads, all synchronized via atomics (no locks in audio path):

```
[App] → [BlackHole] → Input callback → RingBuffer → Output callback → [Speakers]
                                            ↑
                                     TUI thread (main)
                                     reads state / sends commands
```

**`audio/ring_buffer.rs`** — Custom SPSC ring buffer using `UnsafeCell<f32>` + atomic positions. Unlike standard SPSC queues, supports random-access reads (needed for seeking). Positions are absolute/monotonic; physical index = `pos % capacity`.

**`audio/engine.rs`** — Sets up CPAL input/output streams. Finds devices by case-insensitive name substring. Input callback writes to ring buffer; output callback reads from it via the controller.

**`playback/controller.rs`** — The bridge between TUI and audio threads. All fields are atomics. Manages state transitions, seek clamping, anti-click fade ramp (256 samples), and peak level tracking (float×1000→usize for atomic storage, exponential decay 0.85).

**`playback/state.rs`** — Three states: `Live` (read tracks write at base_delay), `Paused` (read frozen, write continues), `TimeShifted` (both advance, read behind by variable amount).

**`tui/`** — ratatui + crossterm. `app.rs` has the event loop (~30fps poll). `ui.rs` renders status, buffer gauge, level meters, device info, keybindings.

## Key Constraints

- Audio callbacks are real-time: no allocations, no locks, no blocking calls
- Ring buffer safety invariant: write region and read region never overlap (enforced by capacity > max delay)
- Sample rates must match between input (BlackHole) and output device; no resampler yet
