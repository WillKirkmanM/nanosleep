use clap::Parser;
use std::ffi::c_long;
use std::process::ExitCode;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(version, about = "Pause for a duration through Linux nanosleep")]
struct Arguments {
    #[arg(value_parser = parse_duration, help = "Duration with an explicit ns, us, ms or s suffix")]
    duration: Duration,

    #[arg(long, help = "Report monotonic elapsed time in nanoseconds")]
    elapsed: bool,
}

fn parse_duration(input: &str) -> Result<Duration, String> {
    let (digits, multiplier) = if let Some(digits) = input.strip_suffix("ns") {
        (digits, 1_u128)
    } else if let Some(digits) = input.strip_suffix("us") {
        (digits, 1_000)
    } else if let Some(digits) = input.strip_suffix("ms") {
        (digits, 1_000_000)
    } else if let Some(digits) = input.strip_suffix('s') {
        (digits, 1_000_000_000)
    } else {
        return Err("duration must have an explicit ns, us, ms or s suffix".into());
    };

    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(
            "duration must contain a non-negative whole number followed by its suffix".into(),
        );
    }

    let overflow = || "duration exceeds the supported nanosleep range".to_owned();
    let value = digits.bytes().try_fold(0_u128, |value, byte| {
        value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u128::from(byte - b'0')))
            .ok_or_else(overflow)
    })?;
    let nanoseconds = value.checked_mul(multiplier).ok_or_else(overflow)?;
    let seconds = nanoseconds / 1_000_000_000;
    if seconds > c_long::MAX as u128 {
        return Err(overflow());
    }

    Ok(Duration::new(
        seconds as u64,
        (nanoseconds % 1_000_000_000) as u32,
    ))
}

#[cfg(target_os = "linux")]
fn sleep(duration: Duration) -> Result<(), String> {
    use std::io;

    #[repr(C)]
    struct Timespec {
        seconds: c_long,
        nanoseconds: c_long,
    }

    unsafe extern "C" {
        fn nanosleep(request: *const Timespec, remaining: *mut Timespec) -> std::ffi::c_int;
    }

    let mut request = Timespec {
        seconds: c_long::try_from(duration.as_secs())
            .map_err(|_| "duration exceeds the supported nanosleep range".to_owned())?,
        nanoseconds: c_long::from(duration.subsec_nanos()),
    };

    loop {
        let mut remaining = Timespec {
            seconds: 0,
            nanoseconds: 0,
        };
        let result = unsafe { nanosleep(&request, &mut remaining) };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(4) {
            return Err(format!("nanosleep failed: {error}"));
        }
        request = remaining;
    }
}

#[cfg(not(target_os = "linux"))]
fn sleep(_duration: Duration) -> Result<(), String> {
    Err("nanosleep is supported only on Linux".into())
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    let started = Instant::now();
    match sleep(arguments.duration) {
        Ok(()) => {
            if arguments.elapsed {
                println!("Elapsed: {} ns", started.elapsed().as_nanos());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("nanosleep: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_units_and_normalises_nanoseconds() {
        for (input, expected) in [
            ("0ns", Duration::ZERO),
            ("7ns", Duration::from_nanos(7)),
            ("12us", Duration::from_micros(12)),
            ("1500ms", Duration::new(1, 500_000_000)),
            ("2s", Duration::from_secs(2)),
            ("1000000001ns", Duration::new(1, 1)),
        ] {
            assert_eq!(parse_duration(input), Ok(expected), "{input}");
        }
    }

    #[test]
    fn rejects_malformed_durations() {
        for input in [
            "", "invalid", "1", "ns", "-1s", "+1s", "1.5s", "1 s", " 1s", "1s ", "1NS", "1m", "１s",
        ] {
            assert!(parse_duration(input).is_err(), "{input}");
        }
    }

    #[test]
    fn checks_supported_range_and_integer_overflow() {
        let maximum = c_long::MAX as u128;
        assert_eq!(
            parse_duration(&format!("{maximum}s")),
            Ok(Duration::from_secs(c_long::MAX as u64))
        );
        assert!(parse_duration(&format!("{}s", maximum + 1)).is_err());
        assert!(parse_duration(&format!("{}s", u128::MAX)).is_err());
        assert!(parse_duration(&format!("{}0ns", u128::MAX)).is_err());
    }
}
