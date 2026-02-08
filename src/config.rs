use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "shifter", version, about = "TUI audio time-shift tool")]
pub struct CliArgs {
    /// Input device name or substring (e.g. "BlackHole")
    #[arg(short, long, default_value = "BlackHole")]
    pub input_device: String,

    /// Output device name or substring (default: system default)
    #[arg(short, long)]
    pub output_device: Option<String>,

    /// Buffer duration in seconds
    #[arg(short, long, default_value_t = 60)]
    pub buffer_seconds: u32,

    /// Base latency/delay in milliseconds (0 = pass-through)
    #[arg(short = 'd', long, default_value_t = 0.0)]
    pub latency_ms: f32,

    /// List available audio devices and exit
    #[arg(short, long)]
    pub list_devices: bool,
}
