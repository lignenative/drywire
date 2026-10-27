//! `drywire-cli`, the command-line client: it creates spaces and
//! invitation links on a Drywire instance, joins a room, captures, sends,
//! receives, mixes and plays, and prints what the network and the jitter
//! buffers are doing. See README.md.

mod account;
mod args;
mod devices;
mod home;
mod join;

use std::process::ExitCode;

const USAGE: &str = "\
Usage: drywire-cli <command> [options]

Commands:
  identity          Print this device's public key (made on first use)
  create-space      Create a space on an instance (--instance, --name)
  invite            Print an invitation link to a space
  accept LINK       Become a member of the space a link is for
  spaces            List the spaces this device knows
  join              Join a room and talk (see options below)
  rename NAME       Change your display name
  export            Print what the instances store about you, as JSON
  leave-space       Leave a space and forget its key
  delete-identity   Delete your identity from the instances, and forget it
  devices           Show the default audio devices, their buffers and latencies

Every command but devices takes --home DIR: where the identity and the
space keys are kept (default $DRYWIRE_HOME, else ~/.drywire).

create-space options:
  --instance URL       The instance, such as https://voice.example.org
  --name NAME          The space's name
  --room NAME          Its first room (default general)
  --display-name NAME  Your name, if this is your first space there

invite options:
  --space NAME         Which space (by name or id; default: the only one)
  --uses N             How many people can use it (default 10, at most 100)
  --days D             How long it lasts (default 7, at most 30)

accept options:
  --display-name NAME  Your name, if you are new on that instance

rename, export, delete-identity options:
  --instance URL       Only this instance (default: every one you know)

join options:
  --space NAME         Which space (by name or id; default: the only one)
  --room NAME          Which room (default: the first)
  --link LINK          Accept this invitation first, then join its space
  --display-name NAME  With --link: your name, if you are new there
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

";

fn main() -> ExitCode {
    let mut argv = std::env::args().skip(1);
    let command = argv.next();
    let rest: Vec<String> = argv.collect();
    let result = match command.as_deref() {
        Some("join") => args::Join::parse(&rest).and_then(join::run),
        Some("devices") => args::Devices::parse(&rest).and_then(devices::run),
        Some("identity") => args::Account::parse(&rest, None).and_then(account::identity),
        Some("create-space") => args::Account::parse(&rest, None).and_then(account::create_space),
        Some("invite") => args::Account::parse(&rest, None).and_then(account::invite),
        Some("accept") => args::Account::parse(&rest, Some("LINK")).and_then(account::accept),
        Some("spaces") => args::Account::parse(&rest, None).and_then(account::spaces),
        Some("rename") => args::Account::parse(&rest, Some("NAME")).and_then(account::rename),
        Some("export") => args::Account::parse(&rest, None).and_then(account::export),
        Some("leave-space") => args::Account::parse(&rest, None).and_then(account::leave_space),
        Some("delete-identity") => {
            args::Account::parse(&rest, None).and_then(account::delete_identity)
        }
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
