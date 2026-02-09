mod audio;
mod config;
mod playback;
mod tui;

use anyhow::Result;
use clap::Parser;

use crate::audio::engine::{list_all_devices, AudioEngine};
use crate::config::CliArgs;
use crate::tui::app::App;

fn main() -> Result<()> {
    let args = CliArgs::parse();

    if args.list_devices {
        return list_all_devices(&args.input_device);
    }

    // Initialize audio engine
    let engine = AudioEngine::new(&args)?;

    eprintln!(
        "Audio: {} -> {} ({}ch {}Hz, {}s buffer, {:.0}ms delay)",
        engine.input_device_name,
        engine.output_device_name,
        engine.channels,
        engine.sample_rate,
        args.buffer_seconds,
        args.latency_ms,
    );

    // Set up panic hook to restore terminal
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = ratatui::restore();
        default_hook(info);
    }));

    // Initialize terminal
    let mut terminal = ratatui::init();

    let mut app = App::new(
        engine.controller.clone(),
        engine.input_device_name.clone(),
        engine.output_device_name.clone(),
        engine.sample_rate,
        engine.channels,
        args.buffer_seconds,
    );

    let result = app.run(&mut terminal);

    // Restore terminal
    ratatui::restore();

    result
}
