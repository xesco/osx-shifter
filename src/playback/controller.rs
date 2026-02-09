use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::audio::ring_buffer::AudioRingBuffer;
use crate::playback::state::PlaybackState;

/// Number of samples for the anti-click fade-in ramp after seeking.
const RAMP_LENGTH: usize = 256;

/// Shared state bridge between the TUI thread and the audio callbacks.
///
/// Seeking model: the TUI sets a `target_delay_samples` and the output callback
/// positions the read head at `wp - target_delay` every cycle. This avoids all
/// races between the TUI thread and the audio callback over the read position.
///
/// - Live:        target_delay = 0 (callback uses its own minimum = one buffer)
/// - TimeShifted: target_delay > 0 (the user-requested extra delay)
/// - Paused:      read head frozen, write continues
pub struct PlaybackController {
    pub ring: Arc<AudioRingBuffer>,
    state: AtomicU8,
    channels: u16,
    sample_rate: u32,
    /// The user-requested delay beyond the minimum callback buffer.
    /// In Live mode this is 0. Seek adds/subtracts from this.
    target_delay_samples: AtomicUsize,
    /// Remaining samples in the anti-click fade-in ramp.
    ramp_remaining: AtomicUsize,
    /// Peak level for left channel, stored as value * 1000.
    peak_left: AtomicUsize,
    /// Peak level for right channel, stored as value * 1000.
    peak_right: AtomicUsize,
    /// Output volume as value * 1000 (1000 = 100%).
    volume: AtomicUsize,
    /// Saved volume before mute (0 = not muted).
    muted_volume: AtomicUsize,
    /// Delay in samples as last computed by the output callback.
    /// Single atomic — no read/write race, so the TUI gets a stable value.
    display_delay_samples: AtomicUsize,
}

impl PlaybackController {
    pub fn new(ring: Arc<AudioRingBuffer>, channels: u16, sample_rate: u32) -> Self {
        Self {
            ring,
            state: AtomicU8::new(PlaybackState::Live as u8),
            channels,
            sample_rate,
            target_delay_samples: AtomicUsize::new(0),
            ramp_remaining: AtomicUsize::new(0),
            peak_left: AtomicUsize::new(0),
            peak_right: AtomicUsize::new(0),
            volume: AtomicUsize::new(1000),
            muted_volume: AtomicUsize::new(0),
            display_delay_samples: AtomicUsize::new(0),
        }
    }

    // -- State queries (called by TUI) --

    pub fn state(&self) -> PlaybackState {
        PlaybackState::from_u8(self.state.load(Ordering::Acquire))
    }

    pub fn delay_ms(&self) -> f64 {
        let delay_samples = self.display_delay_samples.load(Ordering::Relaxed);
        let frames = delay_samples / self.channels as usize;
        frames as f64 / self.sample_rate as f64 * 1000.0
    }

    pub fn buffer_usage(&self) -> f64 {
        self.ring.usage_fraction()
    }

    pub fn peak_levels(&self) -> (f32, f32) {
        let l = self.peak_left.load(Ordering::Relaxed) as f32 / 1000.0;
        let r = self.peak_right.load(Ordering::Relaxed) as f32 / 1000.0;
        (l, r)
    }

    pub fn volume(&self) -> f32 {
        self.volume.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn is_muted(&self) -> bool {
        self.muted_volume.load(Ordering::Relaxed) > 0
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
                // Resume from where we paused: set target to the accumulated delay.
                let actual_delay = self.ring.delay_samples();
                self.target_delay_samples
                    .store(actual_delay, Ordering::Release);
                if actual_delay == 0 {
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
        let delta_samples =
            (delta_ms / 1000.0 * self.sample_rate as f64) as i64 * self.channels as i64;
        let cap = self.ring.capacity() as i64;

        let current = self.target_delay_samples.load(Ordering::Relaxed) as i64;
        // Don't seek further back than what's been written
        let max_delay = (self.ring.write_position() as i64).min(cap);
        let new_target = (current + delta_samples).clamp(0, max_delay);

        self.target_delay_samples
            .store(new_target as usize, Ordering::Release);
        self.ramp_remaining
            .store(RAMP_LENGTH * self.channels as usize, Ordering::Release);

        if new_target == 0 {
            self.state
                .store(PlaybackState::Live as u8, Ordering::Release);
        } else {
            self.state
                .store(PlaybackState::TimeShifted as u8, Ordering::Release);
        }
    }

    pub fn adjust_volume(&self, delta: i32) {
        let current = self.volume.load(Ordering::Relaxed) as i32;
        let new_vol = (current + delta).clamp(0, 1500) as usize;
        self.volume.store(new_vol, Ordering::Relaxed);
        // Unmute on manual volume change
        self.muted_volume.store(0, Ordering::Relaxed);
    }

    pub fn toggle_mute(&self) {
        let saved = self.muted_volume.load(Ordering::Relaxed);
        if saved > 0 {
            // Unmute: restore saved volume
            self.volume.store(saved, Ordering::Relaxed);
            self.muted_volume.store(0, Ordering::Relaxed);
        } else {
            // Mute: save current volume, set to 0
            let current = self.volume.load(Ordering::Relaxed);
            self.muted_volume.store(current.max(1), Ordering::Relaxed);
            self.volume.store(0, Ordering::Relaxed);
        }
    }

    pub fn jump_to_live(&self) {
        self.target_delay_samples.store(0, Ordering::Release);
        self.state
            .store(PlaybackState::Live as u8, Ordering::Release);
        self.ramp_remaining
            .store(RAMP_LENGTH * self.channels as usize, Ordering::Release);
    }

    // -- Called by output callback --

    /// Positions the read head and returns the current state.
    ///
    /// The callback owns the read position. The TUI only sets `target_delay_samples`
    /// and this method translates that into `rp = wp - min_delay - target_delay`.
    pub fn pre_read(&self, frame_count: usize) -> PlaybackState {
        let state = self.state();
        if state == PlaybackState::Paused {
            self.display_delay_samples
                .store(self.ring.delay_samples(), Ordering::Relaxed);
            return state;
        }

        let wp = self.ring.write_position();
        let callback_samples = frame_count * self.channels as usize;
        let target = self.target_delay_samples.load(Ordering::Relaxed);

        // Total delay = one callback buffer (minimum) + user-requested extra delay
        let total_delay = callback_samples + target;

        // Don't go further back than the buffer allows or what's been written
        let clamped = total_delay.min(self.ring.capacity()).min(wp);
        let target_rp = wp.saturating_sub(clamped);
        self.ring.set_read_position(target_rp);

        self.display_delay_samples.store(target, Ordering::Relaxed);
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
            if let Some(&r) = frame.get(1).filter(|_| ch >= 2) {
                peak_r = peak_r.max(r.abs());
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
