//! Command-line options, parsed by hand: a few flags do not justify a
//! dependency.

use std::path::PathBuf;
use std::time::Duration;

use drywire_core::gate::GateConfig;
use drywire_core::opus::FrameSize;

/// Options of `join`.
#[derive(Debug)]
pub struct Join {
    pub home: Option<PathBuf>,
    pub space: Option<String>,
    pub room: Option<String>,
    pub link: Option<String>,
    pub display_name: String,
    pub frame: FrameSize,
    pub bitrate: u32,
    pub redundancy: bool,
    pub gate: GateConfig,
    pub buffer: Option<u32>,
    pub min_buffer_ms: f64,
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

/// Options of the commands about identities, spaces and invitations.
#[derive(Debug, Default)]
pub struct Account {
    pub home: Option<PathBuf>,
    pub instance: Option<String>,
    pub space: Option<String>,
    pub name: Option<String>,
    pub room: Option<String>,
    pub display_name: String,
    pub uses: Option<u32>,
    pub days: Option<f64>,
    /// The positional argument, if any (an invitation link, a new name).
    pub value: Option<String>,
}

/// The display name to use when none is given: the system's user name.
pub fn default_display_name() -> String {
    let name = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "player".into());
    name.chars().take(32).collect()
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
            home: None,
            space: None,
            room: None,
            link: None,
            display_name: default_display_name(),
            frame: FrameSize::Ms5,
            bitrate: 64_000,
            redundancy: false,
            gate: GateConfig::default(),
            buffer: None,
            min_buffer_ms: 0.0,
            input: None,
            output: None,
            echo: false,
            duration: None,
            ping: true,
            quiet: false,
        };
        let mut flags = Flags::new(args);
        while let Some(flag) = flags.next_flag() {
            match flag {
                "--home" => join.home = Some(PathBuf::from(flags.value(flag)?)),
                "--space" => join.space = Some(flags.value(flag)?.to_string()),
                "--room" => join.room = Some(flags.value(flag)?.to_string()),
                "--link" => join.link = Some(flags.value(flag)?.to_string()),
                "--display-name" => join.display_name = flags.value(flag)?.to_string(),
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
                "--min-buffer" => {
                    join.min_buffer_ms = flags.parsed(flag)?;
                    if !(0.0..=1000.0).contains(&join.min_buffer_ms) {
                        return Err("--min-buffer must be between 0 and 1000 ms".into());
                    }
                }
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
        if join.link.is_some() && join.space.is_some() {
            return Err("--link already says which space: drop --space".into());
        }
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

impl Account {
    /// Parses the options of a command that takes `positional` (a
    /// description such as "LINK"), or none.
    pub fn parse(args: &[String], positional: Option<&str>) -> Result<Account, String> {
        let mut account = Account {
            display_name: default_display_name(),
            ..Account::default()
        };
        let mut flags = Flags::new(args);
        while let Some(flag) = flags.next_flag() {
            match flag {
                "--home" => account.home = Some(PathBuf::from(flags.value(flag)?)),
                "--instance" => account.instance = Some(flags.value(flag)?.to_string()),
                "--space" => account.space = Some(flags.value(flag)?.to_string()),
                "--name" => account.name = Some(flags.value(flag)?.to_string()),
                "--room" => account.room = Some(flags.value(flag)?.to_string()),
                "--display-name" => account.display_name = flags.value(flag)?.to_string(),
                "--uses" => account.uses = Some(flags.parsed(flag)?),
                "--days" => {
                    let days: f64 = flags.parsed(flag)?;
                    if !(days > 0.0 && days.is_finite()) {
                        return Err("--days must be a positive number".into());
                    }
                    account.days = Some(days);
                }
                other if positional.is_some() && !other.starts_with("--") => {
                    if account.value.is_some() {
                        return Err(format!("only one {} please", positional.unwrap_or("")));
                    }
                    account.value = Some(other.to_string());
                }
                _ => return Err(unknown(flag)),
            }
        }
        if let Some(what) = positional
            && account.value.is_none()
        {
            return Err(format!("this command needs {what}"));
        }
        Ok(account)
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
            "--space",
            "Team",
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
        assert_eq!(join.space.as_deref(), Some("Team"));
        assert_eq!(join.frame, FrameSize::Ms2_5);
        assert!(join.redundancy && join.file_mode());
        assert_eq!(join.duration, Some(Duration::from_millis(1500)));
        assert_eq!(join.gate.open_dbfs, -35.0);
    }

    #[test]
    fn join_refuses_mistakes() {
        for (args, message) in [
            (&["--frame", "3"][..], "not 2.5, 5 or 10"),
            (&["--echo", "--input", "x"], "pick one"),
            (&["--bogus"], "unknown option"),
            (&["--duration", "-1"], "positive"),
            (&["--space"], "needs a value"),
            (&["--link", "l", "--space", "s"], "drop --space"),
        ] {
            let err = Join::parse(&strings(args)).unwrap_err();
            assert!(err.contains(message), "{args:?}: {err}");
        }
    }

    #[test]
    fn account_commands_take_one_positional_value() {
        let a = Account::parse(&strings(&["LINK", "--display-name", "sam"]), Some("LINK")).unwrap();
        assert_eq!(a.value.as_deref(), Some("LINK"));
        assert_eq!(a.display_name, "sam");
        assert!(Account::parse(&strings(&[]), Some("LINK")).is_err());
        assert!(Account::parse(&strings(&["a", "b"]), Some("LINK")).is_err());
        assert!(Account::parse(&strings(&["a"]), None).is_err());
        assert!(Account::parse(&strings(&["--days", "0"]), None).is_err());
    }
}
