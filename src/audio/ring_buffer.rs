use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Result of a read operation on the ring buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadResult {
    /// Samples were successfully read.
    Ok,
    /// Read position was overwritten (too far behind). Jumped forward, output silence.
    Overrun,
    /// Read position is ahead of write position. Output silence.
    Underrun,
}

/// A lock-free ring buffer supporting sequential writes and random-access reads.
///
/// The write side (input callback) always appends samples.
/// The read side (output callback) reads from a position controlled by the
/// playback controller — enabling pause, seek, and time-shifted playback.
///
/// Positions are absolute sample counts (monotonically increasing).
/// Physical index = `absolute_position % capacity`.
pub struct AudioRingBuffer {
    buffer: Box<[UnsafeCell<f32>]>,
    capacity: usize,
    /// Absolute write position (total interleaved samples written since start).
    write_pos: AtomicUsize,
    /// Absolute read position (where the output callback reads next).
    read_pos: AtomicUsize,
    /// Whether the input stream has started writing data.
    active: AtomicBool,
}

// SAFETY: The producer (input callback) and consumer (output callback)
// access different regions of the buffer. The producer writes ahead of the
// consumer, and the buffer is sized to ensure they never overlap.
unsafe impl Send for AudioRingBuffer {}
unsafe impl Sync for AudioRingBuffer {}

impl AudioRingBuffer {
    /// Create a new ring buffer with the given capacity in interleaved samples.
    pub fn new(capacity: usize) -> Self {
        let mut buf = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buf.push(UnsafeCell::new(0.0));
        }
        Self {
            buffer: buf.into_boxed_slice(),
            capacity,
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
            active: AtomicBool::new(false),
        }
    }

    /// Called by the input callback. Writes interleaved samples into the buffer.
    pub fn write(&self, data: &[f32]) {
        let wp = self.write_pos.load(Ordering::Relaxed);
        for (i, &sample) in data.iter().enumerate() {
            let idx = (wp + i) % self.capacity;
            // SAFETY: only the producer writes; consumer reads at a different
            // region guaranteed by the capacity constraint.
            unsafe {
                *self.buffer[idx].get() = sample;
            }
        }
        self.write_pos.store(wp + data.len(), Ordering::Release);
        self.active.store(true, Ordering::Relaxed);
    }

    /// Called by the output callback. Reads `output.len()` samples starting
    /// at the current `read_pos` and advances `read_pos`.
    pub fn read(&self, output: &mut [f32]) -> ReadResult {
        if !self.active.load(Ordering::Relaxed) {
            for s in output.iter_mut() {
                *s = 0.0;
            }
            return ReadResult::Underrun;
        }

        let rp = self.read_pos.load(Ordering::Acquire);
        let wp = self.write_pos.load(Ordering::Acquire);

        // Overrun: data at read_pos was already overwritten
        if wp > rp + self.capacity {
            let new_rp = wp.saturating_sub(self.capacity / 2);
            self.read_pos.store(new_rp, Ordering::Release);
            for s in output.iter_mut() {
                *s = 0.0;
            }
            return ReadResult::Overrun;
        }

        // Underrun: trying to read ahead of write
        if rp + output.len() > wp {
            for s in output.iter_mut() {
                *s = 0.0;
            }
            return ReadResult::Underrun;
        }

        for (i, sample) in output.iter_mut().enumerate() {
            let idx = (rp + i) % self.capacity;
            // SAFETY: producer writes ahead; this region is stable.
            unsafe {
                *sample = *self.buffer[idx].get();
            }
        }
        self.read_pos.store(rp + output.len(), Ordering::Release);
        ReadResult::Ok
    }

    /// Returns the current absolute write position.
    pub fn write_position(&self) -> usize {
        self.write_pos.load(Ordering::Acquire)
    }

    /// Returns the current absolute read position.
    pub fn read_position(&self) -> usize {
        self.read_pos.load(Ordering::Acquire)
    }

    /// Sets the read position. Called by the controller on seek/jump-to-live.
    pub fn set_read_position(&self, pos: usize) {
        self.read_pos.store(pos, Ordering::Release);
    }

    /// Returns the delay in samples: `write_pos - read_pos`.
    pub fn delay_samples(&self) -> usize {
        let wp = self.write_pos.load(Ordering::Acquire);
        let rp = self.read_pos.load(Ordering::Acquire);
        wp.saturating_sub(rp)
    }

    /// Returns how much of the buffer is currently used (0.0 - 1.0).
    pub fn usage_fraction(&self) -> f64 {
        let delay = self.delay_samples();
        delay as f64 / self.capacity as f64
    }

    /// Returns whether the input has started writing.
    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// Returns the buffer capacity in samples.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read() {
        let rb = AudioRingBuffer::new(1024);
        let input = [1.0_f32, 2.0, 3.0, 4.0];
        rb.write(&input);

        let mut output = [0.0_f32; 4];
        let result = rb.read(&mut output);
        assert_eq!(result, ReadResult::Ok);
        assert_eq!(output, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn underrun_before_write() {
        let rb = AudioRingBuffer::new(1024);
        let mut output = [0.0_f32; 4];
        let result = rb.read(&mut output);
        assert_eq!(result, ReadResult::Underrun);
        assert_eq!(output, [0.0; 4]);
    }

    #[test]
    fn wrap_around() {
        let rb = AudioRingBuffer::new(8);
        let input = [1.0_f32, 2.0, 3.0, 4.0, 5.0, 6.0];
        rb.write(&input);

        let mut output = [0.0_f32; 6];
        rb.read(&mut output);
        assert_eq!(output, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);

        // Write more, wrapping around
        let input2 = [7.0_f32, 8.0, 9.0, 10.0];
        rb.write(&input2);

        let mut output2 = [0.0_f32; 4];
        let result = rb.read(&mut output2);
        assert_eq!(result, ReadResult::Ok);
        assert_eq!(output2, [7.0, 8.0, 9.0, 10.0]);
    }

    #[test]
    fn seek_position() {
        let rb = AudioRingBuffer::new(1024);
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        rb.write(&input);

        // Seek to position 50
        rb.set_read_position(50);
        let mut output = [0.0_f32; 4];
        rb.read(&mut output);
        assert_eq!(output, [50.0, 51.0, 52.0, 53.0]);
    }

    #[test]
    fn delay_samples_tracking() {
        let rb = AudioRingBuffer::new(1024);
        let input = [0.0_f32; 100];
        rb.write(&input);
        assert_eq!(rb.delay_samples(), 100);

        let mut output = [0.0_f32; 30];
        rb.read(&mut output);
        assert_eq!(rb.delay_samples(), 70);
    }
}
