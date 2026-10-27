//! Command-line options, parsed by hand: a few flags do not justify a
//! dependency.

use std::path::PathBuf;
use std::time::Duration;

use drywire_core::gate::GateConfig;
use drywire_core::opus::FrameSize;

/// Options of `join`.
#[derive(Debug)]
pub struct Join {
    pub config: PathBuf,
    pub relay: Option<String>,
    pub frame: FrameSize,
    pub bitrate: u32,
    pub redundancy: bool,
    pub gate: GateConfig,
    pub buffer: Option<u32>,
    pub input: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub echo: bool,
    pub duration: Option<Duration>,
    pub ping: bool,
    pub quiet: bool,
}

/// Options of `devices`.
#[derive(Debug)]
pub struct Devices {
    pub buffer: Option<u32>,
    pub play_silence: bool,
}

/// Options of `new-room`.
#[derive(Debug)]
pub struct NewRoom {
    pub relay: String,
    pub members: Vec<String>,
    pub out: PathBuf,
    pub room: String,
    pub first_session: u32,
    /// Hidden: write the public development files, with fixed keys.
    pub dev: bool,
}

/// Walks the arguments: `--flag` or `--flag value`.
struct Flags<'a> {
    args: std::slice::Iter<'a, String>,
}

impl<'a> Flags<'a> {
    fn new(args: &'a [String]) -> Flags<'a> {
        Flags { args: args.iter() }
    }

    fn next_flag(&mut self) -> Option<&'a str> {
        self.args.next().map(String::as_str)
    }

    fn value(&mut self, flag: &str) -> Result<&'a str, String> {
        self.args
            .next()
            .map(String::as_str)
            .ok_or_else(|| format!("{flag} needs a value"))
    }

    fn parsed<T: std::str::FromStr>(&mut self, flag: &str) -> Result<T, String> {
        let value = self.value(flag)?;
        value
            .parse()
            .map_err(|_| format!("{flag}: {value:?} is not a valid value"))
    }
}

fn unknown(flag: &str) -> String {
    format!("unknown option {flag:?}")
}

impl Join {
    pub fn parse(args: &[String]) -> Result<Join, String> {
        let mut join = Join {
            config: PathBuf::new(),
            relay: None,
            frame: FrameSize::Ms5,
            bitrate: 64_000,
            redundancy: false,
            gate: GateConfig::default(),
            buffer: None,
            input: None,
            output: None,
            echo: false,
            duration: None,
            ping: true,
            quiet: false,
        };
        let mut config = None;
        let mut flags = Flags::new(args);
        while let Some(flag) = flags.next_flag() {
            match flag {
                "--config" => config = Some(PathBuf::from(flags.value(flag)?)),
                "--relay" => join.relay = Some(flags.value(flag)?.to_string()),
                "--frame" => {
                    join.frame = match flags.value(flag)? {
                        "2.5" => FrameSize::Ms2_5,
                        "5" => FrameSize::Ms5,
                        "10" => FrameSize::Ms10,
                        other => return Err(format!("--frame: {other:?} is not 2.5, 5 or 10")),
                    }
                }
                "--bitrate" => join.bitrate = flags.parsed(flag)?,
                "--redundancy" => join.redundancy = true,
                "--gate-open" => join.gate.open_dbfs = flags.parsed(flag)?,
                "--gate-close" => join.gate.close_dbfs = flags.parsed(flag)?,
                "--hold" => join.gate.hold_ms = flags.parsed(flag)?,
                "--buffer" => join.buffer = Some(flags.parsed(flag)?),
                "--input" => join.input = Some(PathBuf::from(flags.value(flag)?)),
                "--output" => join.output = Some(PathBuf::from(flags.value(flag)?)),
                "--echo" => join.echo = true,
                "--duration" => {
                    let seconds: f64 = flags.parsed(flag)?;
                    if !(seconds > 0.0 && seconds.is_finite()) {
                        return Err("--duration must be a positive number of seconds".into());
                    }
                    join.duration = Some(Duration::from_secs_f64(seconds));
                }
                "--no-ping" => join.ping = false,
                "--quiet" => join.quiet = true,
                _ => return Err(unknown(flag)),
            }
        }
        join.config = config.ok_or("join needs --config FILE")?;
        if join.echo && join.input.is_some() {
            return Err("--echo and --input both say what to capture: pick one".into());
        }
        if join.gate.close_dbfs > join.gate.open_dbfs {
            return Err("--gate-close must not be above --gate-open".into());
        }
        Ok(join)
    }

    /// Whether to run without a sound card.
    pub fn file_mode(&self) -> bool {
        self.input.is_some() || self.output.is_some() || self.echo
    }
}

impl Devices {
    pub fn parse(args: &[String]) -> Result<Devices, String> {
        let mut devices = Devices {
            buffer: None,
            play_silence: false,
        };
        let mut flags = Flags::new(args);
        while let Some(flag) = flags.next_flag() {
            match flag {
                "--buffer" => devices.buffer = Some(flags.parsed(flag)?),
                "--play-silence" => devices.play_silence = true,
                _ => return Err(unknown(flag)),
            }
        }
        Ok(devices)
    }
}

impl NewRoom {
    pub fn parse(args: &[String]) -> Result<NewRoom, String> {
        let mut relay = None;
        let mut members = None;
        let mut out = None;
        let mut room = "team".to_string();
        let mut first_session = 1;
        let mut dev = false;
        let mut flags = Flags::new(args);
        while let Some(flag) = flags.next_flag() {
            match flag {
                "--relay" => relay = Some(flags.value(flag)?.to_string()),
                "--members" => {
                    let names: Vec<String> = flags
                        .value(flag)?
                        .split(',')
                        .map(|n| n.trim().to_string())
                        .collect();
                    if names.iter().any(String::is_empty) {
                        return Err("--members: empty name".into());
                    }
                    members = Some(names);
                }
                "--out" => out = Some(PathBuf::from(flags.value(flag)?)),
                "--room" => room = flags.value(flag)?.to_string(),
                "--first-session" => first_session = flags.parsed(flag)?,
                "--dev" => dev = true,
                _ => return Err(unknown(flag)),
            }
        }
        let members = members.ok_or("new-room needs --members A,B,...")?;
        let mut unique = members.clone();
        unique.sort();
        unique.dedup();
        if unique.len() != members.len() {
            return Err("--members: a name is listed twice".into());
        }
        if first_session == 0 {
            return Err("--first-session: session 0 is not valid".into());
        }
        Ok(NewRoom {
            relay: relay.ok_or("new-room needs --relay HOST:PORT")?,
            members,
            out: out.ok_or("new-room needs --out DIR")?,
            room,
            first_session,
            dev,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn join_reads_its_options() {
        let join = Join::parse(&strings(&[
            "--config",
            "a.json",
            "--frame",
            "2.5",
            "--redundancy",
            "--input",
            "in.wav",
            "--duration",
            "1.5",
            "--gate-open",
            "-35",
        ]))
        .unwrap();
        assert_eq!(join.frame, FrameSize::Ms2_5);
        assert!(join.redundancy && join.file_mode());
        assert_eq!(join.duration, Some(Duration::from_millis(1500)));
        assert_eq!(join.gate.open_dbfs, -35.0);
    }

    #[test]
    fn join_refuses_mistakes() {
        for (args, message) in [
            (&["--frame", "5"][..], "--config"),
            (&["--config", "a", "--frame", "3"], "not 2.5, 5 or 10"),
            (&["--config", "a", "--echo", "--input", "x"], "pick one"),
            (&["--config", "a", "--bogus"], "unknown option"),
            (&["--config", "a", "--duration", "-1"], "positive"),
            (&["--config"], "needs a value"),
        ] {
            let err = Join::parse(&strings(args)).unwrap_err();
            assert!(err.contains(message), "{args:?}: {err}");
        }
    }

    #[test]
    fn new_room_needs_unique_members() {
        let err = NewRoom::parse(&strings(&[
            "--relay",
            "r:1",
            "--members",
            "a,a",
            "--out",
            "d",
        ]))
        .unwrap_err();
        assert!(err.contains("twice"), "{err}");
    }
}
