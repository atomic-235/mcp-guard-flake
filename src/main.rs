//! CLI: `mcp-guard --server NAME [--policy FILE] [--audit FILE] -- COMMAND...`

use mcp_guard::{Client, Guard, Policy};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

const USAGE: &str = "usage: mcp-guard --server NAME [--policy FILE] [--audit FILE] -- COMMAND [ARG...]";

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut server: Option<String> = None;
    let mut policy_path: Option<PathBuf> = None;
    let mut audit_path: Option<PathBuf> = None;
    let mut command: Vec<String> = Vec::new();

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--" => {
                command = args[index + 1..].to_vec();
                break;
            }
            "--server" => match args.get(index + 1) {
                Some(value) => {
                    server = Some(value.clone());
                    index += 2;
                }
                None => return usage(),
            },
            "--policy" => match args.get(index + 1) {
                Some(value) => {
                    policy_path = Some(PathBuf::from(value));
                    index += 2;
                }
                None => return usage(),
            },
            "--audit" => match args.get(index + 1) {
                Some(value) => {
                    audit_path = Some(PathBuf::from(value));
                    index += 2;
                }
                None => return usage(),
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            _ => return usage(),
        }
    }

    if command.is_empty() {
        return usage();
    }

    let server = server.unwrap_or_else(|| "mcp".to_owned());
    let policy_path = policy_path.or_else(|| default_policy_path(&server));
    // A missing or unreadable policy must never take the client down with it:
    // warn and relay unguarded. The gateway is a preference filter, not the
    // security boundary.
    let policy = match policy_path {
        Some(path) => match Policy::load(&path) {
            Ok(policy) => policy,
            Err(error) => {
                eprintln!("mcp-guard: {error}; running unguarded");
                Policy::default()
            }
        },
        None => {
            eprintln!("mcp-guard: no policy path; running unguarded");
            Policy::default()
        }
    };

    let audit = audit_path
        .or_else(default_audit_path)
        .and_then(|path| open_audit(&path));

    let mut child = match Command::new(&command[0])
        .args(&command[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            eprintln!("mcp-guard: cannot start {}: {error}", command[0]);
            return 127;
        }
    };

    let guard = Arc::new(Mutex::new(
        Guard::new(policy).with_audit(server, audit),
    ));
    let out = Arc::new(Mutex::new(std::io::stdout()));

    let mut child_in = child.stdin.take().expect("child stdin");
    let child_out = child.stdout.take().expect("child stdout");

    let upstream = Arc::clone(&guard);
    let upstream_out = Arc::clone(&out);
    let to_server = std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let (outcome, replies) = {
                let mut guard = upstream.lock().expect("guard");
                let outcome = guard.client(&line);
                (outcome, guard.take_outbox())
            };
            for reply in replies {
                write_line(&upstream_out, &reply);
            }
            match outcome {
                Client::Forward(line) => {
                    if writeln!(child_in, "{line}").and_then(|()| child_in.flush()).is_err() {
                        break;
                    }
                }
                Client::Block(reply) => {
                    if !reply.is_empty() {
                        write_line(&upstream_out, &reply);
                    }
                }
            }
        }
        let _ = child_in.flush();
    });

    let downstream = Arc::clone(&guard);
    let downstream_out = Arc::clone(&out);
    let from_server = std::thread::spawn(move || {
        let reader = BufReader::new(child_out);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let relayed = {
                let mut guard = downstream.lock().expect("guard");
                guard.server(&line)
            };
            if let Some(line) = relayed {
                write_line(&downstream_out, &line);
            }
        }
    });

    let status = child.wait();
    let _ = from_server.join();
    drop(to_server);

    match status {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("mcp-guard: {error}");
            1
        }
    }
}

fn usage() -> i32 {
    eprintln!("{USAGE}");
    2
}

fn write_line(out: &Arc<Mutex<std::io::Stdout>>, line: &str) {
    let mut out = out.lock().expect("stdout");
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn config_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
}

fn state_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME") {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
}

fn default_policy_path(server: &str) -> Option<PathBuf> {
    config_home().map(|dir| dir.join("opencode/mcp-guard").join(format!("{server}.json")))
}

fn default_audit_path() -> Option<PathBuf> {
    state_home().map(|dir| dir.join("mcp-guard/audit.jsonl"))
}

fn open_audit(path: &PathBuf) -> Option<std::fs::File> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => Some(file),
        Err(error) => {
            eprintln!("mcp-guard: audit disabled ({path:?}): {error}");
            None
        }
    }
}
