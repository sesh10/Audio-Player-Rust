// use anyhow::{Context, Result};
// use rodio::{OutputStream, Sink};
// use std::fs::File;

// use symphonia::core::audio::SampleBuffer;
// use symphonia::core::codecs::DecoderOptions;
// use symphonia::core::formats::FormatOptions;
// use symphonia::core::io::MediaSourceStream;
// use symphonia::core::meta::MetadataOptions;
// use symphonia::default::get_probe;
use anyhow::{Context, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use rodio::{buffer::SamplesBuffer, OutputStream, Sink};
use std::fs::File;
use std::path::PathBuf;
use std::thread;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::default::get_probe;

// Play an audio file directly (blocking). This is a simple utility function for testing and debugging.
pub fn play_audio(file_path: &str) -> Result<()> {
    // Create audio output
    let (_stream, handle) = OutputStream::try_default()?;
    let sink = Sink::try_new(&handle)?;

    // Open file
    let file = File::open(file_path).context("Failed to open file")?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    // Probe the file format
    let probed = get_probe().format(
        &Default::default(),
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut format = probed.format;

    // Use default track (usually first audio track)
    let track = format.default_track().context("No default track found")?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())?;

    // Decode and stream into rodio
    while let Ok(packet) = format.next_packet() {
        let decoded = decoder.decode(&packet)?;
        let spec = *decoded.spec();
        let duration = decoded.capacity() as u64;

        // Convert to interleaved samples
        let mut samples = SampleBuffer::<i16>::new(duration, spec);
        samples.copy_interleaved_ref(decoded.clone());

        // Turn into a rodio Source
        let source = rodio::buffer::SamplesBuffer::new(
            decoded.spec().channels.count() as u16,
            decoded.spec().rate,
            samples.samples().to_vec(),
        );

        sink.append(source);
    }

    println!("Playing: {}", file_path);

    sink.sleep_until_end();
    Ok(())
}


// Commands the UI can send to the audio thread.
pub enum PlayerCommand {
    // Load and start playing the given file.
    Play(PathBuf),
    // Pause playback.
    Pause,
    // Resume playback.
    Resume,
    // Stop playback and clear queue.
    Stop,
    // Shutdown thread.
    Quit,
}

/// Start the audio thread. Returns a Sender you can use to control it.
///
/// The audio thread keeps the OutputStream alive and manages a single Sink.
/// Each `Play` replaces the current Sink and starts a new one.
pub fn start_player_thread() -> Sender<PlayerCommand> {
    let (tx, rx): (Sender<PlayerCommand>, Receiver<PlayerCommand>) = unbounded();

    thread::spawn(move || {
        // Keep the stream alive for the whole thread lifetime.
        let (_stream, handle) = match OutputStream::try_default() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Audio output initialization failed: {e}");
                return;
            }
        };

        let mut current_sink: Option<Sink> = None;

        while let Ok(cmd) = rx.recv() {
            match cmd {
                PlayerCommand::Play(path) => {
                    // Stop old sink if any
                    if let Some(s) = current_sink.take() {
                        s.stop();
                    }
                    // New sink
                    let Ok(sink) = Sink::try_new(&handle) else {
                        eprintln!("Failed to create audio sink");
                        continue;
                    };

                    // Decode and enqueue buffers
                    if let Err(err) = decode_and_enqueue(&path, &sink) {
                        eprintln!("Failed to play {:?}: {err}", path);
                        // ensure sink is dropped
                        sink.stop();
                        current_sink = None;
                        continue;
                    }

                    // Keep sink; it's already playing by default.
                    current_sink = Some(sink);
                }
                PlayerCommand::Pause => {
                    if let Some(ref s) = current_sink {
                        s.pause();
                    }
                }
                PlayerCommand::Resume => {
                    if let Some(ref s) = current_sink {
                        s.play();
                    }
                }
                PlayerCommand::Stop => {
                    if let Some(s) = current_sink.take() {
                        s.stop();
                    }
                }
                PlayerCommand::Quit => {
                    if let Some(s) = current_sink.take() {
                        s.stop();
                    }
                    break;
                }
            }
        }
    });

    tx
}

// Try to estimate duration (ms) from codec params if possible.
// Returns None if not available. (UI will handle None gracefully.)
pub fn estimate_duration_ms(path: &PathBuf) -> Option<u64> {
    let file = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let probed = get_probe()
        .format(
            &Default::default(),
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let format = probed.format;

    let track = format.default_track()?;
    let cp = &track.codec_params;

    // If we have both sample_rate and n_frames, we can estimate duration.
    if let (Some(sr), Some(nf)) = (cp.sample_rate, cp.n_frames) {
        if sr > 0 {
            return Some((nf as u128 * 1000u128 / sr as u128) as u64);
        }
    }
    None
}

// Decode the file with symphonia and enqueue raw PCM buffers into the sink.
fn decode_and_enqueue(path: &PathBuf, sink: &Sink) -> Result<()> {
    let file = File::open(path).with_context(|| format!("open file {:?}", path))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let probed = get_probe().format(
        &Default::default(),
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut format = probed.format;

    let track = format
        .default_track()
        .context("No default audio track in file")?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())?;

    // Decode packets and feed to rodio.
    while let Ok(packet) = format.next_packet() {
        let decoded = decoder.decode(&packet)?;
        let spec = *decoded.spec();
        let duration = decoded.capacity() as u64;

        let mut samples = SampleBuffer::<i16>::new(duration, spec);
        samples.copy_interleaved_ref(decoded.clone());

        let src = SamplesBuffer::new(
            decoded.spec().channels.count() as u16,
            decoded.spec().rate,
            samples.samples().to_vec(),
        );
        sink.append(src);
    }

    // Start playing (Sink starts in play state, but calling play() is harmless)
    sink.play();
    Ok(())
}