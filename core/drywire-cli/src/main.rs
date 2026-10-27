//! `drywire-cli`, the phase 1 client: it joins a room on a relay, captures,
//! sends, receives, mixes and plays, and prints what the network and the
//! jitter buffers are doing. See README.md.

mod args;
mod devices;
mod join;
mod room;

use std::process::ExitCode;

const USAGE: &str = "\
Usage: drywire-cli <command> [options]

Commands:
  join       Join a room and talk (see options below)
  devices    Show the default audio devices, their buffers and latencies
  new-room   Write a relay rooms file and one client configuration per member

join options:
  --config FILE        Client configuration file (required)
  --relay HOST:PORT    Use this relay instead of the one in the file
  --frame MS           Opus frame size: 2.5, 5 or 10 (default 5)
  --bitrate BPS        Opus bitrate (default 64000)
  --redundancy         Also send the previous frame in each packet
  --gate-open DBFS     Level that opens the voice gate (default -40)
  --gate-close DBFS    Level under which it starts closing (default -46)
  --hold MS            How long it stays open after that (default 150)
  --buffer FRAMES      Ask the sound card for this I/O buffer (default: smallest)
  --min-buffer MS      Never let the jitter buffers aim below this delay
                       (default 0: one frame, or more if the network asks)
  --input FILE.wav     Capture from a 48 kHz WAV file instead of the microphone
  --output FILE.wav    Play into a WAV file instead of the speakers
  --echo               Send back what you receive (an echo bot; no sound card)
  --duration SECONDS   Stop after this long (file mode: input length + 1 s)
  --no-ping            Do not ping the relay (keepalives only; a relay
                       restart then goes unnoticed until you join again)
  --quiet              Print only the start and the final summary

devices options:
  --buffer FRAMES      Show what asking for this buffer would give
  --play-silence       Also play one second of silence (macOS), to check the
                       output path and see the callback sizes the OS uses

new-room options:
  --relay HOST:PORT    The relay's public address (required)
  --members A,B,...    Member names, one configuration each (required)
  --out DIR            Where to write the files (required)
  --room NAME          Room name in the rooms file (default team)
  --first-session N    Session of the first member (default 1)
";

fn main() -> ExitCode {
    let mut argv = std::env::args().skip(1);
    let command = argv.next();
    let rest: Vec<String> = argv.collect();
    let result = match command.as_deref() {
        Some("join") => args::Join::parse(&rest).and_then(join::run),
        Some("devices") => args::Devices::parse(&rest).and_then(devices::run),
        Some("new-room") => args::NewRoom::parse(&rest).and_then(room::run),
        Some("help" | "-h" | "--help") | None => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => Err(format!("unknown command {other:?}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("drywire-cli: {message}");
            eprintln!("Run `drywire-cli help` for the options.");
            ExitCode::FAILURE
        }
    }
}
