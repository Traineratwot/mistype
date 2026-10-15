//! `switchfix setup` — interactive privileged setup.
//!
//! Asks before every action. `sudo` (and its password prompt) is invoked
//! only for steps that actually need root, and only after confirmation.

use std::io::{self, Write};
use std::process::{Command, Stdio};

use tracing::info;

pub struct SetupReport {
    pub done: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
    pub need_relogin: bool,
}

impl SetupReport {
    pub fn print(&self) {
        if !self.done.is_empty() {
            println!("\nDone:");
            for d in &self.done {
                println!("  [OK] {d}");
            }
        }
        if !self.skipped.is_empty() {
            println!("\nSkipped:");
            for s in &self.skipped {
                println!("  [--] {s}");
            }
        }
        if !self.failed.is_empty() {
            println!("\nFailed:");
            for f in &self.failed {
                println!("  [!!] {f}");
            }
        }
        if self.need_relogin {
            println!("\nGroup membership changed — log out and back in for it to take effect.");
        }
        if self.failed.is_empty() {
            println!("\nSetup finished. Next: switchfix doctor");
        } else {
            std::process::exit(1);
        }
    }
}

fn ask_yes_no(prompt: &str, default_yes: bool) -> bool {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("{prompt} {hint} ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        return default_yes;
    }
    match line.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        "" => default_yes,
        _ => default_yes,
    }
}

fn is_root() -> bool {
    run_raw("id", &["-u"])
        .map(|u| u.trim() == "0")
        .unwrap_or(false)
}

/// Run a command as root via `sudo` (prompts for password when needed).
/// Returns Ok(stdout) or Err(stderr).
fn run_sudo(args: &[&str]) -> Result<String, String> {
    if is_root() {
        return run_raw(args[0], &args[1..]);
    }
    eprintln!("  (sudo) {}", args.join(" "));
    // No `-k`: reuse a valid sudo ticket; sudo still prompts when the
    // ticket is missing or expired.
    let out = Command::new("sudo")
        .arg("--")
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to run sudo: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn run_raw(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run {cmd}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn in_input_group_session() -> bool {
    run_raw("id", &["-nG"])
        .map(|g| g.split_whitespace().any(|x| x == "input"))
        .unwrap_or(false)
}

/// Check the passwd database (`id -nG "$USER"`), not just the current session —
/// after `usermod` the membership is visible there before re-login.
fn in_input_group_passwd() -> bool {
    let user = std::env::var("USER").unwrap_or_default();
    let args: Vec<&str> = if user.is_empty() {
        vec!["-nG"]
    } else {
        vec!["-nG", &user]
    };
    run_raw("id", &args)
        .map(|g| g.split_whitespace().any(|x| x == "input"))
        .unwrap_or(false)
}

fn udev_rule_installed() -> bool {
    std::path::Path::new("/etc/udev/rules.d/99-switchfix-uinput.rules").exists()
}

fn udev_rule_source() -> Option<std::path::PathBuf> {
    // Prefer the source tree next to the binary / cwd; fall back to packaged path.
    let candidates = [
        std::path::PathBuf::from("packaging/systemd/99-switchfix-uinput.rules"),
        std::path::PathBuf::from("/usr/share/switchfix/99-switchfix-uinput.rules"),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("../share/switchfix/99-switchfix-uinput.rules")))
            .unwrap_or_default(),
    ];
    candidates.into_iter().find(|p| p.exists())
}

/// Interactive setup. `assume_yes` answers "yes" to all prompts (still uses sudo).
pub fn run(assume_yes: bool) -> SetupReport {
    let mut report = SetupReport {
        done: vec![],
        skipped: vec![],
        failed: vec![],
        need_relogin: false,
    };

    println!("SwitchFix setup");
    println!("===============");
    println!("Privileged steps will ask for your sudo password only if you confirm them.\n");

    // ---- 1. input group (required for global hotkeys + keystroke buffer) ----
    if in_input_group_passwd() {
        if in_input_group_session() {
            report.done.push("already in group 'input'".into());
        } else {
            report.done.push(
                "user is in group 'input' (re-login needed for this session)".into(),
            );
            report.need_relogin = true;
        }
    } else {
        println!("Your user is NOT in the 'input' group.");
        println!("  Needed for: global hotkeys (Ctrl+F1…) and the keystroke buffer.");
        println!("  Risk: any 'input'-group process can read raw key events — see docs/SECURITY.md.");
        let ok = assume_yes || ask_yes_no("Add your user to the 'input' group now?", true);
        if !ok {
            report.skipped.push("group 'input' not changed".into());
        } else {
            let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
            match run_sudo(&["usermod", "-aG", "input", &user]) {
                Ok(_) => {
                    report.done.push(format!("added {user} to group 'input'"));
                    report.need_relogin = true;
                }
                Err(e) => report.failed.push(format!("usermod input: {e}")),
            }
        }
    }

    // ---- 2. udev rule for /dev/uinput (optional convenience) ----
    if udev_rule_installed() {
        report.done.push("udev rule already installed".into());
    } else {
        println!("\nOptional udev rule: makes /dev/uinput group-accessible (MODE=0660, GROUP=input).");
        let ok = assume_yes || ask_yes_no("Install udev rule (needs sudo)?", true);
        if !ok {
            report.skipped.push("udev rule not installed".into());
        } else {
            match udev_rule_source() {
                None => report.failed.push(
                    "udev rule source not found (packaging/systemd/99-switchfix-uinput.rules)".into(),
                ),
                Some(src) => {
                    let src_s = src.to_string_lossy().to_string();
                    let dst = "/etc/udev/rules.d/99-switchfix-uinput.rules";
                    // Copy via a root shell so we don't depend on install(1) flags.
                    let cmd = format!(
                        "install -m 0644 {} {} && udevadm control --reload-rules && udevadm trigger --subsystem-match=input || true",
                        sh_quote(&src_s),
                        dst
                    );
                    match run_sudo(&["sh", "-c", &cmd]) {
                        Ok(_) => report.done.push(format!("installed udev rule → {dst}")),
                        Err(e) => report.failed.push(format!("udev rule: {e}")),
                    }
                }
            }
        }
    }

    // ---- 3. user service (no sudo) ----
    println!("\nUser service: `systemctl --user enable --now switchfix` (no sudo needed).");
    let ok = assume_yes || ask_yes_no("Enable and start the SwitchFix service now?", true);
    if ok {
        match run_raw("systemctl", &["--user", "enable", "--now", "switchfix"]) {
            Ok(_) => report.done.push("user service enabled and started".into()),
            Err(e) => {
                // Unit may not be installed yet — that's not a hard failure.
                report
                    .skipped
                    .push(format!("user service not started ({e})"));
            }
        }
    } else {
        report.skipped.push("user service left disabled".into());
    }

    info!(target: "switchfix", "setup done={:?} skipped={:?} failed={:?}", report.done, report.skipped, report.failed);
    report
}

fn sh_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_'))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}
