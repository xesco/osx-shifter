# Shifter — Audio Time-Shift Tool

## Context

The goal is to build a macOS TUI tool in Rust that acts as a DVR for audio. Audio from any app (browser, streaming) is routed through a virtual audio device (BlackHole), captured by Shifter, buffered in a ring buffer, and output to physical speakers with user-controllable time-shifting: pause, rewind, fast-forward, and jump-to-live — all with millisecond-precision delay display.

```
[Any App] → [BlackHole (virtual device)] → [Shifter] → [Speakers/Headphones]
```

## Prerequisites

- **BlackHole** installed (free, open-source virtual audio driver): `brew install blackhole-2ch`
- User sets their app/system audio output to BlackHole

## Architecture

### Module Structure

```
shifter/
  Cargo.toml
  src/
    main.rs                 # Entry point, CLI parsing, orchestration
    audio/
      mod.rs
      engine.rs             # CPAL stream setup, device selection
      ring_buffer.rs        # Lock-free ring buffer with random-access reads
    playback/
      mod.rs
      state.rs              # PlaybackState enum and transitions
      controller.rs         # Shared atomic state between audio & TUI threads
    tui/
      mod.rs
      app.rs                # Event loop, keyboard handling
      ui.rs                 # Ratatui widget rendering
    config.rs               # CLI args (clap)
```

### Dependencies (`Cargo.toml`)

```toml
[dependencies]
cpal = "0.15"
ratatui = "0.29"
crossterm = "0.28"
clap = { version = "4", features = ["derive"] }
anyhow = "1"
```

### Core Components

#### 1. Ring Buffer (`audio/ring_buffer.rs`)

Custom lock-free SPSC ring buffer — unlike standard SPSC queues, this supports **random-access reads** (needed for seeking/rewinding).

- Backing store: `Box<[UnsafeCell<f32>]>` with `capacity` interleaved samples
- **Absolute positions** via `AtomicUsize` for `write_pos` and `read_pos` (monotonically increasing, physical index = `pos % capacity`)
- `write(&self, data: &[f32])` — producer (input callback), Release ordering
- `read(&self, output: &mut [f32]) -> ReadResult` — consumer (output callback), Acquire ordering
- `set_read_position(&self, pos: usize)` — called by controller on seek
- Overrun detection (writer laps reader → jump reader forward, output silence)
- Underrun detection (reader ahead of writer → output silence)
- Buffer size: at 48kHz stereo, 60s = ~23MB

#### 2. Playback State Machine (`playback/state.rs`)

```
States: Live | Paused | TimeShifted

Live ──pause──→ Paused ──resume──→ TimeShifted ──jump_to_live──→ Live
                                   TimeShifted ──pause──→ Paused
Live ──seek_back──→ TimeShifted
Any  ──jump_to_live──→ Live
```

- **Live**: read follows write at `base_delay` offset, both advance
- **Paused**: write continues, read frozen — buffer fills up
- **TimeShifted**: both advance, read is behind write by variable amount

#### 3. Playback Controller (`playback/controller.rs`)

Shared state bridge using only atomics (no locks — real-time safe):
- `state: AtomicU8` — current playback state
- `base_delay_samples: AtomicUsize`
- `ramp_remaining: AtomicUsize` — anti-click fade-in counter (256 samples) after seeks
- `peak_level_left/right: AtomicUsize` — for VU meters
- Methods: `toggle_pause()`, `seek_ms(delta)`, `jump_to_live()`, `delay_ms() -> f64`

#### 4. Audio Engine (`audio/engine.rs`)

- Uses CPAL with CoreAudio backend
- Finds input device by name substring (default: "BlackHole")
- Output device: system default or specified by name
- Enforces matching sample rate between input/output (errors with instructions if mismatch)
- Input callback: `ring.write(data)`
- Output callback: `controller.advance_read()` then `ring.read(output)`, applies fade ramp if seeking, updates peak levels

#### 5. TUI (`tui/ui.rs`, `tui/app.rs`)

```
┌──────────────────── Shifter ─────────────────────┐
│  State: ▶ LIVE       Delay: 150ms    Buffer: 12% │
├───────────────────────────────────────────────────┤
│  ████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  │
│  ^read                                  write^    │
├───────────────────────────────────────────────────┤
│  L ▕████████████▏ -42 dB                          │
│  R ▕██████████▏   -45 dB                          │
├───────────────────────────────────────────────────┤
│  In:  BlackHole 2ch 48000Hz                       │
│  Out: MacBook Pro Speakers 48000Hz                │
├───────────────────────────────────────────────────┤
│  Space:pause ←→:seek 5s ⇧←→:30s L:live Q:quit    │
└───────────────────────────────────────────────────┘
```

- ~30 FPS refresh via `event::poll(Duration::from_millis(33))`
- Panic hook restores terminal

### Thread Model (3 threads, pure atomics)

```
Main thread (TUI)          Input callback (CPAL)     Output callback (CPAL)
  │                            │                         │
  │ reads state for display    │ ring.write(data)        │ ctrl.advance_read()
  │ sends commands (atomics)   │ updates write_pos       │ ring.read(output)
  │                            │                         │ ctrl.update_peaks()
```

No mutexes, no channels. All communication via atomic loads/stores on the shared `PlaybackController`.

### Keyboard Controls

| Key | Action |
|-----|--------|
| `Space` | Pause / Resume |
| `←` | Seek back 5s |
| `→` | Seek forward 5s |
| `Shift+←` | Seek back 30s |
| `Shift+→` | Seek forward 30s |
| `L` | Jump to live |
| `Q` | Quit |

### CLI

```bash
shifter --list-devices                    # show available audio devices
shifter                                   # defaults: BlackHole in, system out, 60s buffer, 150ms delay
shifter -i "BlackHole" -d 200 -b 120      # custom input, 200ms delay, 120s buffer
```

## Implementation Order

| Step | What | Testable Result |
|------|------|-----------------|
| 1 | Project scaffold + CLI + `--list-devices` | Verify CPAL sees BlackHole |
| 2 | Ring buffer + unit tests | Tests pass (write, read, wrap, overrun, underrun) |
| 3 | Audio passthrough (engine) | Hear audio through BlackHole → speakers with fixed delay |
| 4 | Playback controller + state machine | Pause/seek/live via simple CLI print |
| 5 | TUI shell | Full interactive interface |
| 6 | Polish: anti-click ramp, peak meters, error handling | Smooth seek, visual feedback |

## Verification

1. `cargo build --release` compiles cleanly
2. `shifter --list-devices` shows BlackHole and output device
3. Play audio in browser with output set to BlackHole
4. `shifter` shows audio levels, delay counter, state=LIVE
5. Press Space → state=PAUSED, delay increases in real-time, audio stops
6. Press Space → state=TIME_SHIFTED, audio resumes from where it paused
7. Press ← → delay increases by 5s, audio jumps back (no click)
8. Press → → delay decreases, audio jumps forward
9. Press L → state=LIVE, delay returns to base delay
10. Press Q → clean terminal restore, audio stops
