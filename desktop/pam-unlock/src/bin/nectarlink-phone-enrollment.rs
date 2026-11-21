// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("nectarlink-phone-enrollment: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(target_os = "linux")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        fs::OpenOptions,
        io::{self, BufRead, BufReader, Read, Write},
        path::Path,
    };

    use pam_nectarlink_phone_unlock::{
        DEFAULT_ENROLLMENT_DIR, Enrollment, EnrollmentStore, account_name, require_root,
    };

    require_root()?;
    let mut args = std::env::args();
    args.next();
    match (args.next().as_deref(), args.next(), args.next()) {
        (Some("enroll"), None, None) => {
            let enrollment = Enrollment::read(io::stdin().lock())?;
            let account = account_name(enrollment.uid)?;
            let store = EnrollmentStore::open(Path::new(DEFAULT_ENROLLMENT_DIR))?;
            let mut tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
            writeln!(tty, "Linux account {account:?} (UID {})", enrollment.uid)?;
            writeln!(tty, "PC {}", enrollment.pc)?;
            writeln!(tty, "Phone {}", enrollment.phone)?;
            let fingerprint = enrollment.fingerprint();
            let groups =
                fingerprint.as_bytes().chunks(8).map(std::str::from_utf8).collect::<Result<Vec<_>, _>>()?;
            writeln!(tty, "Public key SHA-256 {}", groups.join(" "))?;
            write!(tty, "Enter the fingerprint from your phone to authorize enrollment: ")?;
            tty.flush()?;
            let mut confirmation = String::new();
            BufReader::new(&tty).take(128).read_line(&mut confirmation)?;
            confirmation.retain(|character| character != ' ');
            store.enroll(&enrollment, confirmation.trim())?;
            writeln!(tty, "Phone unlock enrollment saved.")?;
        }
        (Some("revoke"), Some(uid), None) => {
            let uid: u32 = uid.parse()?;
            EnrollmentStore::open(Path::new(DEFAULT_ENROLLMENT_DIR))?.revoke(uid)?;
            println!("Phone unlock enrollment removed for UID {uid}.");
        }
        _ => return Err("usage: nectarlink-phone-enrollment enroll < enrollment.json | revoke <uid>".into()),
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("NectarLink phone unlock enrollment requires Linux-PAM.");
    std::process::ExitCode::FAILURE
}
