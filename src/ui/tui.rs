// use std::io;
// use crossterm::{
//     event::{self, Event, KeyCode},
//     terminal::{disable_raw_mode, enable_raw_mode},
// };
// use ratatui::{
//     backend::CrosstermBackend,
//     layout::{Constraint, Direction, Layout},
//     style::{Color, Style},
//     text::{Span},
//     widgets::{Block, Borders, Gauge, List, ListItem, Paragraph},
//     Terminal,
// };

// /// Run the TUI
// pub fn run_tui() -> anyhow::Result<()> {
//     enable_raw_mode()?;
//     let mut stdout = io::stdout();
//     let backend = CrosstermBackend::new(&mut stdout);
//     let mut terminal = Terminal::new(backend)?;

//     let playlist = vec!["song1.mp3", "song2.flac", "song3.ogg"];
//     let mut current_index = 0;
//     let mut progress: f64 = 0.0;

//     loop {
//         terminal.draw(|f| {
//             // Split screen
//             let chunks = Layout::default()
//                 .direction(Direction::Vertical)
//                 .margin(1)
//                 .constraints([
//                     Constraint::Length(3),
//                     Constraint::Length(3),
//                     Constraint::Min(5),
//                 ])
//                 .split(f.size());

//             // Now Playing
//             let now_playing = Paragraph::new(format!("🎵 Now Playing: {}", playlist[current_index]))
//                 .block(Block::default().borders(Borders::ALL).title("Now Playing"));
//             f.render_widget(now_playing, chunks[0]);

//             // Progress bar
//             let gauge = Gauge::default()
//                 .block(Block::default().borders(Borders::ALL).title("Progress"))
//                 .gauge_style(Style::default().fg(Color::Green))
//                 .ratio(progress);
//             f.render_widget(gauge, chunks[1]);

//             // Playlist
//             let items: Vec<ListItem> = playlist
//                 .iter()
//                 .enumerate()
//                 .map(|(i, song)| {
//                     if i == current_index {
//                         ListItem::new(Span::styled(
//                             format!("> {}", song),
//                             Style::default().fg(Color::Yellow),
//                         ))
//                     } else {
//                         ListItem::new(song.to_string())
//                     }
//                 })
//                 .collect();

//             let list = List::new(items)
//                 .block(Block::default().borders(Borders::ALL).title("Playlist"));
//             f.render_widget(list, chunks[2]);
//         })?;

//         // Handle input
//         if event::poll(std::time::Duration::from_millis(200))? {
//             if let Event::Key(key) = event::read()? {
//                 match key.code {
//                     KeyCode::Char('q') => break, // Quit
//                     KeyCode::Char('n') => { // Next track
//                         current_index = (current_index + 1) % playlist.len();
//                         progress = 0.0;
//                     }
//                     KeyCode::Char('p') => { // Previous track
//                         if current_index == 0 {
//                             current_index = playlist.len() - 1;
//                         } else {
//                             current_index -= 1;
//                         }
//                         progress = 0.0;
//                     }
//                     KeyCode::Char(' ') => {
//                         // TODO: hook into pause/resume backend
//                     }
//                     _ => {}
//                 }
//             }
//         }

//         // Simulate progress increasing (demo only)
//         progress += 0.01;
//         if progress > 1.0 {
//             progress = 0.0;
//             current_index = (current_index + 1) % playlist.len();
//         }
//     }

//     disable_raw_mode()?;
//     Ok(())
// }

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    execute,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::Span,
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph},
    Terminal,
};

use crate::player::audio::{estimate_duration_ms, start_player_thread, PlayerCommand};

const TICK: Duration = Duration::from_millis(150);

pub fn run_tui() -> Result<()> {
    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Gather playlist from current directory
    let mut playlist = collect_audio_files(".")?;
    if playlist.is_empty() {
        // Fallback message, then cleanly restore terminal
        disable_and_restore(&mut terminal)?;
        eprintln!("No audio files found in current directory. Supported: mp3, flac, wav, ogg, m4a");
        return Ok(());
    }

    let tx = start_player_thread();

    // UI state
    let mut current_index: usize = 0;
    let mut is_playing = false;
    let mut is_paused = false;
    let mut is_looped  = false;

    let mut elapsed_ms: u64 = 0;
    let mut total_ms: Option<u64> = estimate_duration_ms(&playlist[current_index]);

    let mut last_tick = Instant::now();

    // Autoplay first item on load
    tx.send(PlayerCommand::Play(playlist[current_index].clone())).ok();
    is_playing = true;
    is_paused = false;
    elapsed_ms = 0;

    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(5),
                ])
                .split(f.size());

            // Now Playing
            let now_playing_text = format!(
                "🎵 {}{}",
                playlist[current_index]
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<unknown>"),
                if is_looped { "  🔁" } else { "" }
            );
            let now_playing = Paragraph::new(now_playing_text)
                .block(Block::default().borders(Borders::ALL).title("Now Playing"));
            f.render_widget(now_playing, chunks[0]);

            // Progress
            let ratio = match total_ms {
                Some(total) if total > 0 => (elapsed_ms as f64 / total as f64).clamp(0.0, 1.0),
                _ => 0.0,
            };
            let time_text = match total_ms {
                Some(total) => format!("{}/{}", fmt_ms(elapsed_ms), fmt_ms(total)),
                None => format!("{} / --:--", fmt_ms(elapsed_ms)),
            };
            let gauge = Gauge::default()
                .block(Block::default().borders(Borders::ALL).title("Progress"))
                .gauge_style(Style::default().fg(Color::Green))
                .ratio(ratio)
                .label(time_text);
            f.render_widget(gauge, chunks[1]);

            // Playlist list
            let items: Vec<ListItem> = playlist
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("<file>");
                    if i == current_index {
                        ListItem::new(Span::styled(
                            format!("> {}", name),
                            Style::default().fg(Color::Yellow),
                        ))
                    } else {
                        ListItem::new(name.to_string())
                    }
                })
                .collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(
                    "Playlist  (↑/↓ select, Enter play, Space pause/resume, l Loop, n/p next/prev, s stop, q quit)",
                ));
            f.render_widget(list, chunks[2]);
        })?;

        // Tick: update elapsed time if playing and not paused
        let now = Instant::now();
        if now.duration_since(last_tick) >= TICK {
            last_tick = now;
            if is_playing && !is_paused {
                elapsed_ms = elapsed_ms.saturating_add(TICK.as_millis() as u64);
                if let Some(total) = total_ms {
                    if elapsed_ms >= total {
                        if is_looped {
                            // Restart current track
                            send_play(&tx, &playlist[current_index]);
                            elapsed_ms = 0;
                            total_ms = estimate_duration_ms(&playlist[current_index]);
                        } else {
                            // Auto advance to next track
                            next_track(&mut current_index, playlist.len());
                            send_play(&tx, &playlist[current_index]);
                            is_playing = true;
                            is_paused = false;
                            elapsed_ms = 0;
                            total_ms = estimate_duration_ms(&playlist[current_index]);
                        }
                    }
                }
            }
        }

        // Handle input (non-blocking poll)
        if event::poll(TICK)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => {
                        tx.send(PlayerCommand::Quit).ok();
                        break;
                    }
                    KeyCode::Up => {
                        if current_index > 0 {
                            current_index -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if current_index + 1 < playlist.len() {
                            current_index += 1;
                        }
                    }
                    KeyCode::Enter => {
                        send_play(&tx, &playlist[current_index]);
                        is_playing = true;
                        is_paused = false;
                        elapsed_ms = 0;
                        total_ms = estimate_duration_ms(&playlist[current_index]);
                    }
                    KeyCode::Char(' ') => {
                        if is_playing && !is_paused {
                            tx.send(PlayerCommand::Pause).ok();
                            is_paused = true;
                        } else if is_playing && is_paused {
                            tx.send(PlayerCommand::Resume).ok();
                            is_paused = false;
                        } else {
                            // Not playing currently; start current selection
                            send_play(&tx, &playlist[current_index]);
                            is_playing = true;
                            is_paused = false;
                            elapsed_ms = 0;
                            total_ms = estimate_duration_ms(&playlist[current_index]);
                        }
                    }
                    KeyCode::Char('l') => {
                        is_looped = !is_looped;
                    }
                    KeyCode::Char('n') => {
                        next_track(&mut current_index, playlist.len());
                        send_play(&tx, &playlist[current_index]);
                        is_playing = true;
                        is_paused = false;
                        elapsed_ms = 0;
                        total_ms = estimate_duration_ms(&playlist[current_index]);
                    }
                    KeyCode::Char('p') => {
                        prev_track(&mut current_index, playlist.len());
                        send_play(&tx, &playlist[current_index]);
                        is_playing = true;
                        is_paused = false;
                        elapsed_ms = 0;
                        total_ms = estimate_duration_ms(&playlist[current_index]);
                    }
                    KeyCode::Char('s') => {
                        tx.send(PlayerCommand::Stop).ok();
                        is_playing = false;
                        is_paused = false;
                        elapsed_ms = 0;
                    }
                    _ => {}
                }
            }
        }
    }

    // Restore terminal
    disable_and_restore(&mut terminal)?;
    Ok(())
}

fn disable_and_restore(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn next_track(index: &mut usize, len: usize) {
    *index = (*index + 1) % len;
}
fn prev_track(index: &mut usize, len: usize) {
    if *index == 0 {
        *index = len - 1;
    } else {
        *index -= 1;
    }
}

fn send_play(tx: &crossbeam_channel::Sender<PlayerCommand>, p: &PathBuf) {
    let _ = tx.send(PlayerCommand::Play(p.clone()));
}

/// Collect audio files (non-recursive) from a directory.
fn collect_audio_files(dir: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let mut v = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file() && is_supported(&path) {
            v.push(path);
        }
    }
    // simple stable sort by filename
    v.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(v)
}

fn is_supported(p: &Path) -> bool {
    match p.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()) {
        Some(ext) if ["mp3", "flac", "wav", "ogg", "m4a"].contains(&ext.as_str()) => true,
        _ => false,
    }
}

fn fmt_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    let m = total_secs / 60;
    let s = total_secs % 60;
    format!("{:02}:{:02}", m, s)
}