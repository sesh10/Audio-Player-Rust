mod player;
mod ui;
mod utils;

use anyhow::Result;

fn main() -> Result<()> {
    utils::logger::init();

    // For now, run the TUI instead of CLI
    ui::tui::run_tui()
}