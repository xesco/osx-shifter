# Shifter

A macOS TUI tool that acts as a DVR for audio. Pause, rewind, and time-shift any audio source in real-time.

```
[Any App] → [BlackHole] → [Shifter] → [Speakers]
```

## Prerequisites

Install [BlackHole](https://existential.audio/blackhole/), a free virtual audio driver:

```bash
brew install blackhole-2ch
```

## Setup

1. Set your Mac audio output to **BlackHole 2ch** (System Settings > Sound > Output)
2. Run Shifter — it captures from BlackHole and outputs to your physical speakers automatically

## Usage

```bash
shifter                              # default: BlackHole in, speakers out
shifter -i "BlackHole" -o "MacBook"  # explicit devices
shifter -b 120 -d 200                # 120s buffer, 200ms base delay
shifter -l                           # list audio devices
```

## Controls

| Key | Action |
|-----|--------|
| `Space` | Pause / Resume |
| `←` / `→` | Seek back / forward by current step |
| `1`-`9` | Set seek step (1=1ms 2=10ms 3=100ms 4=500ms 5=1s 6=2s 7=5s 8=10s 9=30s) |
| `↑` / `↓` | Volume up / down (5% steps) |
| `L` | Jump to live |
| `H` | Toggle help overlay |
| `Q` | Quit |

## Build

```bash
cargo build --release
cargo test
```
