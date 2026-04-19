use clap::Parser;
use std::fmt;

/// CLI client for the Notir WebSocket message server.
#[derive(Debug, Parser)]
#[command(name = "notir-cli", version, about)]
pub struct Cli {
    /// Server URL (e.g. ws://localhost:5800)
    #[arg(long, default_value = "ws://localhost:5800")]
    pub server: String,

    /// User/channel ID to subscribe as
    #[arg(long)]
    pub id: String,

    /// Subscription mode: single (point-to-point) or broad (broadcast)
    #[arg(short, long, default_value = "single")]
    pub mode: SubscriptionMode,

    /// Path to JS transform script. Without this, raw text is passed through.
    #[arg(short, long)]
    pub script: Option<String>,

    /// Output mode: stdout, file, or both
    #[arg(short, long, default_value = "stdout")]
    pub output: OutputMode,

    /// Output directory for file mode
    #[arg(short, long, default_value = "./output")]
    pub output_dir: String,

    /// File writing mode: append to single file or one file per message
    #[arg(long, default_value = "append")]
    pub file_mode: FileMode,

    /// Enable auto-reconnect on disconnect
    #[arg(long)]
    pub reconnect: bool,

    /// Reconnect interval in seconds
    #[arg(long, default_value_t = 3)]
    pub reconnect_interval: u64,

    /// Max reconnect attempts (0 = unlimited)
    #[arg(long, default_value_t = 5)]
    pub max_reconnect: u32,

    /// Enable verbose/debug logging
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SubscriptionMode {
    Single,
    Broad,
}

impl fmt::Display for SubscriptionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubscriptionMode::Single => write!(f, "single"),
            SubscriptionMode::Broad => write!(f, "broad"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputMode {
    Stdout,
    File,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum FileMode {
    Append,
    Individual,
}
