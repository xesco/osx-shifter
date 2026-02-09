# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project Overview

Shifter is a macOS-only TUI audio time-shift (DVR) tool written in Rust (edition 2024, requires rustc 1.85+). It captures audio from a virtual audio device (e.g. BlackHole), buffers it in a lock-free ring buffer, and outputs to physical speakers with user-controllable pause/seek/rewind. The entire audio path is lock-free — all cross-thread communication uses atomics, never `Mutex`, `RwLock`, or channels.

## Build & Test Commands

```bash
cargo build --release           # build optimized binary
cargo build                     # build debug binary
cargo test                      # run all tests
cargo test ring_buffer          # run ring buffer tests only
cargo test write_then_read      # run a single test by name
cargo run --release             # run (requires a virtual audio device installed)
cargo run --release -- -l       # list available audio devices
```

There is no `rustfmt.toml` — standard `rustfmt` defaults apply. Run `cargo fmt` before committing. Run `cargo clippy` to lint.

## Architecture

Three threads, synchronized entirely via atomics:

```
[App] → [Virtual Device] → Input callback → RingBuffer → Output callback → [Speakers]
                                                 ↑
                                          TUI thread (main)
                                          reads state / sends commands
```

### Key Files

| File | Role |
|------|------|
| `src/main.rs` | Entry point: CLI parsing, audio engine init, terminal setup, app loop |
| `src/config.rs` | CLI argument definitions via `clap::Parser` derive |
| `src/audio/engine.rs` | CoreAudio engine: device discovery, AudioUnit setup, input/output callbacks |
| `src/audio/ring_buffer.rs` | Lock-free SPSC ring buffer with random-access reads (the only tested module) |
| `src/playback/controller.rs` | Atomic bridge between TUI and audio: state, seek, volume, ramp, peaks |
| `src/playback/state.rs` | `PlaybackState` enum (`Live`, `Paused`, `TimeShifted`) with `#[repr(u8)]` |
| `src/tui/app.rs` | TUI event loop: key handling, ~30fps polling |
| `src/tui/ui.rs` | Ratatui rendering: status, buffer gauge, level meters, help overlay |

`engine.rs` contains a private inner module `mod coreaudio_device { ... }` that encapsulates all raw CoreAudio FFI calls. This keeps unsafe FFI details isolated from the rest of the codebase.

## Key Constraints

- **Audio callbacks are real-time:** no allocations, no locks, no blocking, no I/O, no panics. Callbacks must always return `Ok(())`.
- **Ring buffer safety invariant:** write and read regions never overlap. Enforced by sizing capacity > max delay.
- **Sample rates must match** between input and output devices. No resampler exists.
- **Device validation:** input device (`-i`) must be a known virtual device (BlackHole, Soundflower, Loopback). Output device (`-o`) must be physical (non-virtual) and different from the input.
- **macOS only:** depends on CoreAudio via `coreaudio-rs` and `coreaudio-sys`.

## Code Style

### Imports

Three groups separated by blank lines, in this order:

```rust
use std::sync::Arc;                              // 1. std

use anyhow::{anyhow, Result};                    // 2. External crates
use clap::Parser;

use crate::audio::engine::AudioEngine;           // 3. Internal (crate::)
use crate::config::CliArgs;
```

Always use fully-qualified `crate::` paths for internal imports. Never use `super::` except in test modules (`use super::*`).

### Formatting

- Standard `rustfmt` defaults (4-space indent, K&R braces).
- Line length under ~100 characters. Break longer expressions across lines.
- Trailing commas in multi-line struct literals, function arguments, and array literals.
- Prefix unused fields with underscore: `_input_unit`, `_base_delay_ms`.
- Explicitly discard unused values: `let _ = modifiers;`.

### Naming

| Kind | Convention | Examples |
|------|-----------|----------|
| Modules | `snake_case` | `ring_buffer`, `controller` |
| Types/Structs | `PascalCase` | `AudioRingBuffer`, `PlaybackController` |
| Enum variants | `PascalCase` | `Live`, `Paused`, `TimeShifted` |
| Functions | `snake_case` | `delay_samples()`, `toggle_pause()` |
| Constants | `UPPER_SNAKE_CASE` | `RAMP_LENGTH`, `VIRTUAL_DEVICE_NAMES` |
| Variables | `snake_case`, short | `wp`, `rp`, `ch`, `peak_l` |

### Type Conventions

- `u32` for sample rates.
- `u16` for channel counts.
- `usize` for buffer sizes and sample positions.
- `f32` for audio sample data.
- `f64` for time calculations and display values.
- `Arc<T>` for shared ownership across threads.
- Atomics for all cross-thread state — never `Mutex` or `RwLock`.

### Atomic Patterns

- `Acquire`/`Release` ordering for data-dependent operations (ring buffer positions, state transitions).
- `Relaxed` ordering for display-only values (peak levels, volume readback).
- `#[repr(u8)]` + `AtomicU8` for storing enums atomically. Convert with a `from_u8()` method that falls back to a default for unknown values.
- Float-as-scaled-integer: store `(float * 1000.0) as usize` in `AtomicUsize`, read back as `value as f32 / 1000.0`. Used for peak levels.

## Error Handling

- Use `anyhow` for all application-level errors. `main()`, `AudioEngine::new()`, and `App::run()` all return `anyhow::Result<()>`.
- Create errors with `anyhow!("message")`. Include user guidance in multi-line messages:
  ```rust
  return Err(anyhow!(
      "'{name}' is not a virtual audio device.\n\
       Use -l to list available input devices."
  ));
  ```
- Convert library errors with `.map_err(|e| anyhow!("context: {e}"))`. Do not use `.context()` or `.with_context()`.
- Convert `Option` to `Result` with `.ok_or_else(|| anyhow!("message"))`.
- **Never use `.unwrap()` or `.expect()` in production code.** Use `unwrap_or_else` with a fallback value instead.
- Audio callbacks must never return errors — always return `Ok(())`.

## Unsafe Code

- Every `unsafe` block in application code must have a preceding `// SAFETY:` comment explaining the invariant.
- `unsafe impl Send/Sync` for the ring buffer is justified by the SPSC access pattern (producer and consumer access disjoint regions).
- Raw CoreAudio FFI is isolated inside the private `mod coreaudio_device` inner module in `engine.rs`. Keep it there.
- Ring buffer uses `UnsafeCell<f32>` for interior mutability without locks. The safety invariant is that read and write regions never overlap.

## Testing

Tests live in `#[cfg(test)] mod tests` blocks inside the source file they test. Convention:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptive_name_without_test_prefix() {
        let buf = AudioRingBuffer::new(1024);
        // ... exercise and assert
        assert_eq!(result, expected);
    }
}
```

- Test names are `snake_case`, descriptive, without a `test_` prefix.
- Each test creates fresh state — no shared fixtures or helper functions.
- Use `assert_eq!` for assertions.
- Only the ring buffer module has tests (it's the only pure-logic, non-hardware module). Audio engine and TUI are hardware-dependent and tested manually.
- Run a single test: `cargo test descriptive_name`.
