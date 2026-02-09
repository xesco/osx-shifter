# Shifter

A macOS terminal tool for syncing a live audio source with a video stream. Adds a precise, adjustable delay to live audio — set it once, leave it running.

![Shifter](shifter.png)

## Why?

You're watching a live football match on a streaming service but prefer your local radio's commentary. Both come over the internet, but the video stream lags behind the radio — the commentator celebrates the goal before you see it.

Shifter delays the radio audio to match the slower video stream. Dial in the delay once, then leave it running for the rest of the match.

```
Audio source → Virtual audio device → Shifter → Speakers
```

## Requirements

- **macOS** (uses CoreAudio directly)
- A virtual audio device such as **[BlackHole](https://existential.audio/blackhole/)**
- **Rust** toolchain ([rustup.rs](https://rustup.rs))

## Install

```bash
brew install blackhole-2ch

git clone https://github.com/xesco/osx-shifter.git
cd osx-shifter
cargo build --release
```

The binary is at `target/release/shifter`. Copy it somewhere on your `PATH` or run directly.

After installing your virtual audio device, set it as the system audio output in **System Settings → Sound → Output**. This routes all system audio through it so Shifter can capture it.

If Shifter reports a sample rate mismatch (e.g. `Sample rate mismatch: input (BlackHole 2ch) = 48000Hz, output (External Headphones) = 44100Hz`), open **Audio MIDI Setup** and set both devices to the same sample rate.

## Usage

```bash
shifter                              # default: BlackHole in, system speakers out
shifter -i "BlackHole" -o "MacBook"  # explicit devices (substring match)
shifter -b 120                       # 120 second buffer
shifter -l                           # list available devices
```

### Options

| Flag | Description | Default |
|------|-------------|---------|
| `-i, --input-device` | Input device name (substring match) | `BlackHole` |
| `-o, --output-device` | Output device name (substring match) | System output |
| `-b, --buffer-seconds` | Ring buffer duration in seconds | `60` |
| `-l, --list-devices` | List available devices and exit | |

### Controls

| Key | Action |
|-----|--------|
| `Space` | Pause / Resume |
| `→` | Seek backward (increase delay) |
| `←` | Seek forward (toward live) |
| `1`-`9` | Seek step: 1ms, 10ms, 100ms, 500ms, 1s, 2s, 5s, 10s, 30s |
| `↑` / `↓` | Volume up/down (5% steps, max 150%) |
| `L` | Jump to live |
| `H` | Toggle help overlay |
| `Q` | Quit |

## How It Works

Three threads, all synchronized via atomics — no locks in the audio path:

- **Input callback** captures from the virtual audio device into a lock-free ring buffer
- **Output callback** reads from the ring buffer to speakers, positioned by a target delay
- **TUI thread** renders the interface and translates key presses into atomic writes

The seeking model is simple: the TUI sets a `target_delay` atomic, and the output callback positions the read head at `write_pos - callback_buffer - target_delay` every cycle. No direct manipulation of the read position from the TUI thread, no races.

**States:**

- **Live** — target=0, pass-through
- **TimeShifted** — target>0
- **Paused** — write continues, read frozen

## Build

```bash
cargo build --release
cargo test
```

## License

[MIT](LICENSE)
