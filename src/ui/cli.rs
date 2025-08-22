use clap::Parser;
use anyhow::Result;

use crate::player::audio;

/// Simple CLI Audio Player
#[derive(Parser, Debug)]
#[command(name = "Audio Player", version, about = "Play audio files from the command line")]
struct Args {
    /// Path to the audio file
    file: String,
}

pub fn run() -> Result<()> {
    let args = Args::parse();
    audio::play_audio(&args.file)
}