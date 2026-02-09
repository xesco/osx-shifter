use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::playback::controller::PlaybackController;
use crate::tui::ui;

/// Seek scales indexed 0..8 corresponding to keys 1..9.
pub const SEEK_SCALES: [(f64, &str); 9] = [
    (1.0, "1ms"),
    (10.0, "10ms"),
    (100.0, "100ms"),
    (500.0, "500ms"),
    (1_000.0, "1s"),
    (2_000.0, "2s"),
    (5_000.0, "5s"),
    (10_000.0, "10s"),
    (30_000.0, "30s"),
];

pub struct App {
    pub controller: Arc<PlaybackController>,
    pub should_quit: bool,
    pub input_device_name: String,
    pub output_device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_seconds: u32,
    /// Current seek scale index (0..8, default 4 = 1s).
    pub seek_scale_index: usize,
    /// Whether the help overlay is shown.
    pub show_help: bool,
}

impl App {
    pub fn new(
        controller: Arc<PlaybackController>,
        input_device_name: String,
        output_device_name: String,
        sample_rate: u32,
        channels: u16,
        buffer_seconds: u32,
    ) -> Self {
        Self {
            controller,
            should_quit: false,
            input_device_name,
            output_device_name,
            sample_rate,
            channels,
            buffer_seconds,
            seek_scale_index: 4, // default: 1s
            show_help: false,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| ui::draw(frame, self))?;

            // Poll at ~30 FPS for smooth meter updates
            if event::poll(Duration::from_millis(33))?
                && let Event::Key(key) = event::read()?
            {
                if key.kind != crossterm::event::KeyEventKind::Press {
                    continue;
                }
                self.handle_key(key.code, key.modifiers);
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.should_quit = true;
            }
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Char(' ') => {
                self.controller.toggle_pause();
            }
            KeyCode::Char('l') | KeyCode::Char('L') => {
                self.controller.jump_to_live();
            }
            KeyCode::Char('h') | KeyCode::Char('H') => {
                self.show_help = !self.show_help;
            }
            KeyCode::Left => {
                let step_ms = SEEK_SCALES[self.seek_scale_index].0;
                self.controller.seek_ms(-step_ms);
            }
            KeyCode::Right => {
                let step_ms = SEEK_SCALES[self.seek_scale_index].0;
                self.controller.seek_ms(step_ms);
            }
            KeyCode::Up => {
                self.controller.adjust_volume(50);
            }
            KeyCode::Down => {
                self.controller.adjust_volume(-50);
            }
            // Number keys 1-9 select seek scale
            KeyCode::Char(c @ '1'..='9') => {
                self.seek_scale_index = (c as usize - '1' as usize).min(8);
            }
            _ => {}
        }
    }
}
