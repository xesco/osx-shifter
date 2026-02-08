use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::audio::ring_buffer::AudioRingBuffer;
use crate::playback::state::PlaybackState;

/// Number of samples for the anti-click fade-in ramp after seeking.
const RAMP_LENGTH: usize = 256;

/// Shared state bridge between the TUI thread and the audio callbacks.
/// All fields are atomics — safe to access from any thread without locks.
pub struct PlaybackController {
    pub ring: Arc<AudioRingBuffer>,
    state: AtomicU8,
    base_delay_samples: AtomicUsize,
    channels: u16,
    sample_rate: u32,
    /// Remaining samples in the anti-click fade-in ramp.
    ramp_remaining: AtomicUsize,
    /// Peak level for left channel, stored as value * 1000.
    peak_left: AtomicUsize,
    /// Peak level for right channel, stored as value * 1000.
    peak_right: AtomicUsize,
    /// Output volume as value * 1000 (1000 = 100%).
    volume: AtomicUsize,
}

impl PlaybackController {
    pub fn new(
        ring: Arc<AudioRingBuffer>,
        channels: u16,
        sample_rate: u32,
        base_delay_ms: f32,
    ) -> Self {
        // Ensure a minimum ~10ms base delay so the output callback always has data
        let min_delay_ms = 10.0_f32;
        let effective_ms = base_delay_ms.max(min_delay_ms);
        let base_delay_samples =
            (effective_ms / 1000.0 * sample_rate as f32) as usize * channels as usize;
        Self {
            ring,
            state: AtomicU8::new(PlaybackState::Live as u8),
            base_delay_samples: AtomicUsize::new(base_delay_samples),
            channels,
            sample_rate,
            ramp_remaining: AtomicUsize::new(0),
            peak_left: AtomicUsize::new(0),
            peak_right: AtomicUsize::new(0),
            volume: AtomicUsize::new(1000),
        }
    }

    // -- State queries (called by TUI) --

    pub fn state(&self) -> PlaybackState {
        PlaybackState::from_u8(self.state.load(Ordering::Acquire))
    }

    pub fn delay_ms(&self) -> f64 {
        let delay_samples = self.ring.delay_samples();
        let frames = delay_samples / self.channels as usize;
        frames as f64 / self.sample_rate as f64 * 1000.0
    }

    #[allow(dead_code)]
    pub fn delay_samples(&self) -> usize {
        self.ring.delay_samples()
    }

    pub fn buffer_usage(&self) -> f64 {
        self.ring.usage_fraction()
    }

    pub fn peak_levels(&self) -> (f32, f32) {
        let l = self.peak_left.load(Ordering::Relaxed) as f32 / 1000.0;
        let r = self.peak_right.load(Ordering::Relaxed) as f32 / 1000.0;
        (l, r)
    }

    #[allow(dead_code)]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    #[allow(dead_code)]
    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn volume(&self) -> f32 {
        self.volume.load(Ordering::Relaxed) as f32 / 1000.0
    }

    #[allow(dead_code)]
    pub fn base_delay_ms(&self) -> f64 {
        let samples = self.base_delay_samples.load(Ordering::Relaxed);
        let frames = samples / self.channels as usize;
        frames as f64 / self.sample_rate as f64 * 1000.0
    }

    // -- Commands (called by TUI) --

    pub fn toggle_pause(&self) {
        let current = self.state();
        match current {
            PlaybackState::Live | PlaybackState::TimeShifted => {
                self.state
                    .store(PlaybackState::Paused as u8, Ordering::Release);
            }
            PlaybackState::Paused => {
                // Check if we're close enough to live
                let base = self.base_delay_samples.load(Ordering::Relaxed);
                let delay = self.ring.delay_samples();
                // Tolerance: within 2 callback buffers worth
                let tolerance = self.sample_rate as usize / 10 * self.channels as usize;
                if delay <= base + tolerance {
                    self.state
                        .store(PlaybackState::Live as u8, Ordering::Release);
                } else {
                    self.state
                        .store(PlaybackState::TimeShifted as u8, Ordering::Release);
                }
                self.ramp_remaining
                    .store(RAMP_LENGTH * self.channels as usize, Ordering::Release);
            }
        }
    }

    pub fn seek_ms(&self, delta_ms: f64) {
        let delta_frames = (delta_ms / 1000.0 * self.sample_rate as f64) as i64;
        let delta_samples = delta_frames * self.channels as i64;

        let rp = self.ring.read_position() as i64;
        let wp = self.ring.write_position() as i64;
        let base = self.base_delay_samples.load(Ordering::Relaxed) as i64;
        let cap = self.ring.capacity() as i64;

        let new_rp = rp - delta_samples; // negative delta = seek backward = smaller rp

        // Clamp: can't read ahead of write minus base_delay
        let max_rp = wp - base;
        // Clamp: can't go further back than capacity allows
        let min_rp = wp - cap + (cap / 10); // leave 10% margin

        let clamped = new_rp.clamp(min_rp, max_rp);
        self.ring.set_read_position(clamped.max(0) as usize);
        self.ramp_remaining
            .store(RAMP_LENGTH * self.channels as usize, Ordering::Release);

        // Update state based on new position
        let delay = (wp - clamped) as usize;
        let tolerance = self.sample_rate as usize / 10 * self.channels as usize;
        if delay <= base as usize + tolerance {
            self.state
                .store(PlaybackState::Live as u8, Ordering::Release);
        } else {
            self.state
                .store(PlaybackState::TimeShifted as u8, Ordering::Release);
        }
    }

    pub fn adjust_volume(&self, delta: f32) {
        let current = self.volume.load(Ordering::Relaxed) as f32 / 1000.0;
        let new_vol = (current + delta).clamp(0.0, 1.5);
        self.volume
            .store((new_vol * 1000.0) as usize, Ordering::Relaxed);
    }

    pub fn jump_to_live(&self) {
        let wp = self.ring.write_position();
        let base = self.base_delay_samples.load(Ordering::Relaxed);
        self.ring
            .set_read_position(wp.saturating_sub(base));
        self.state
            .store(PlaybackState::Live as u8, Ordering::Release);
        self.ramp_remaining
            .store(RAMP_LENGTH * self.channels as usize, Ordering::Release);
    }

    // -- Called by output callback --

    /// Advances the read position if in Live mode (to track the write head).
    /// Returns the current state so the callback knows how to behave.
    pub fn pre_read(&self, frame_count: usize) -> PlaybackState {
        let state = self.state();
        if state == PlaybackState::Live {
            // In live mode, keep read position tracking write - base_delay.
            // Ensure we're at least one callback buffer behind so read() won't underrun.
            let wp = self.ring.write_position();
            let base = self.base_delay_samples.load(Ordering::Relaxed);
            let callback_samples = frame_count * self.channels as usize;
            let effective_delay = base.max(callback_samples);
            let target_rp = wp.saturating_sub(effective_delay);
            self.ring.set_read_position(target_rp);
        }
        state
    }

    /// Applies software volume to the output buffer.
    pub fn apply_volume(&self, data: &mut [f32]) {
        let vol = self.volume.load(Ordering::Relaxed) as f32 / 1000.0;
        if (vol - 1.0).abs() > 0.001 {
            for s in data.iter_mut() {
                *s *= vol;
            }
        }
    }

    /// Applies the anti-click ramp to the output buffer if needed.
    pub fn apply_ramp(&self, data: &mut [f32]) {
        let ramp = self.ramp_remaining.load(Ordering::Acquire);
        if ramp == 0 {
            return;
        }
        let ramp_total = RAMP_LENGTH * self.channels as usize;
        let elapsed = ramp_total.saturating_sub(ramp);
        for (i, sample) in data.iter_mut().enumerate() {
            let pos = elapsed + i;
            if pos < ramp_total {
                let gain = pos as f32 / ramp_total as f32;
                *sample *= gain;
            }
        }
        let consumed = data.len().min(ramp);
        self.ramp_remaining.fetch_sub(consumed, Ordering::Release);
    }

    /// Updates peak levels from the output buffer.
    pub fn update_peaks(&self, data: &[f32]) {
        if self.channels == 0 {
            return;
        }
        let mut peak_l: f32 = 0.0;
        let mut peak_r: f32 = 0.0;
        let ch = self.channels as usize;

        for frame in data.chunks(ch) {
            if let Some(&l) = frame.first() {
                peak_l = peak_l.max(l.abs());
            }
            if ch >= 2 {
                if let Some(&r) = frame.get(1) {
                    peak_r = peak_r.max(r.abs());
                }
            }
        }

        // Exponential decay for smooth meter movement
        let decay = 0.85;
        let prev_l = self.peak_left.load(Ordering::Relaxed) as f32 / 1000.0;
        let prev_r = self.peak_right.load(Ordering::Relaxed) as f32 / 1000.0;
        let new_l = peak_l.max(prev_l * decay);
        let new_r = peak_r.max(prev_r * decay);

        self.peak_left
            .store((new_l * 1000.0) as usize, Ordering::Relaxed);
        self.peak_right
            .store((new_r * 1000.0) as usize, Ordering::Relaxed);
    }
}
