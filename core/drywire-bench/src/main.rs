//! Command line: runs every built-in scenario at every frame size and prints
//! one table per frame size.
//!
//! ```text
//! drywire-bench [simulate] [--seed N]
//! ```

use std::process::ExitCode;

use drywire_bench::scenarios;
use drywire_bench::simulate::{Report, run};
use drywire_core::opus::FrameSize;

const USAGE: &str = "usage: drywire-bench [simulate] [--seed N]";

fn main() -> ExitCode {
    let mut seed = 1;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "simulate" => {}
            "--seed" => match args.next().and_then(|s| s.parse().ok()) {
                Some(s) => seed = s,
                None => {
                    eprintln!("{USAGE}");
                    return ExitCode::FAILURE;
                }
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }

    println!("Simulated network traces against the jitter buffer (seed {seed}).");
    println!("\"added\" is the time a frame waited in the buffer before playing.");
    for frame in FrameSize::ALL {
        println!();
        println!("Frames of {frame}");
        println!(
            "{:<16} {:>7} {:>6} {:>6} {:>6} {:>6} {:>6} {:>7} {:>7} {:>7} {:>7}",
            "scenario",
            "sent",
            "lost",
            "PLC %",
            "drop %",
            "recov",
            "late",
            "target",
            "p50",
            "p95",
            "max"
        );
        for scenario in scenarios::all(frame, seed) {
            print_row(&run(&scenario));
        }
    }
    println!();
    println!("Times in milliseconds. PLC and drop are per frame sent.");
    ExitCode::SUCCESS
}

fn print_row(r: &Report) {
    let ms = |us: u64| us as f64 / 1000.0;
    println!(
        "{:<16} {:>7} {:>6} {:>6.2} {:>6.2} {:>6} {:>6} {:>7.1} {:>7.1} {:>7.1} {:>7.1}",
        r.scenario.name,
        r.sent,
        r.lost,
        r.concealed_percent(),
        r.dropped_percent(),
        r.buffer.recovered,
        r.buffer.late,
        ms(r.buffer.target_us),
        ms(r.added_us[0]),
        ms(r.added_us[1]),
        ms(r.added_us[2]),
    );
}
