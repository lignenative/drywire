//! `drywire devices`: what the default audio devices report. No sound
//! plays. On macOS no stream is opened, so macOS does not ask for the
//! microphone. On Windows each stream is initialized as `join` would, read
//! and released without starting: WASAPI only reports periods and
//! latencies for an initialized client, and exclusive capture briefly
//! takes the microphone.

use crate::args::Devices;
use drywire_core::audio::AudioOptions;

pub fn run(args: Devices) -> Result<(), String> {
    let options = AudioOptions {
        buffer_frames: args.buffer,
    };
    #[cfg(target_os = "macos")]
    {
        for info in drywire_core::audio::coreaudio::probe(options) {
            match info {
                Ok(info) => println!("{info}"),
                Err(e) => println!("{e}"),
            }
        }
        if args.play_silence {
            play_silence(options)?;
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        if args.play_silence {
            return Err("--play-silence is only available on macOS for now".into());
        }
        for info in drywire_core::audio::wasapi::probe(options) {
            match info {
                Ok(info) => println!("{info}"),
                Err(e) => println!("{e}"),
            }
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (options, args.play_silence);
        Err("no audio backend for this operating system yet".into())
    }
}

/// Opens the output for one second of silence and reports how often, and
/// with how many samples, Core Audio called back.
#[cfg(target_os = "macos")]
fn play_silence(options: AudioOptions) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use drywire_core::audio::coreaudio::CoreAudio;

    let calls = Arc::new(AtomicU64::new(0));
    let samples = Arc::new(AtomicU64::new(0));
    let sizes = Arc::new(Mutex::new(std::collections::BTreeSet::new()));
    let (c, s, z) = (Arc::clone(&calls), Arc::clone(&samples), Arc::clone(&sizes));
    let mut audio = CoreAudio::new(options);
    let info = audio
        .start_output(Box::new(move |out: &mut [f32]| {
            out.fill(0.0);
            c.fetch_add(1, Ordering::Relaxed);
            s.fetch_add(out.len() as u64, Ordering::Relaxed);
            if let Ok(mut sizes) = z.try_lock() {
                sizes.insert(out.len());
            }
        }))
        .map_err(|e| e.to_string())?;
    std::thread::sleep(std::time::Duration::from_secs(1));
    drywire_core::audio::AudioIo::stop(&mut audio);
    println!("played silence on {info}");
    println!(
        "  in 1 s: {} callbacks, {} samples at 48 kHz, callback sizes {:?}",
        calls.load(Ordering::Relaxed),
        samples.load(Ordering::Relaxed),
        sizes.lock().expect("not poisoned"),
    );
    Ok(())
}
