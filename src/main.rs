//! Abrasive — a transparent cargo wrapper with remote build support.
//!
//! If the current directory (or any parent) contains an abrasive.toml,
//! cargo commands are forwarded to a remote build server via rsync + ssh.
//! Otherwise, cargo is invoked locally as a transparent passthrough.

use log::{debug, error, info};
use serde::Deserialize;
use std::{
    env,
    fs,
    path::{Path, PathBuf},
    process::{exit, Command, ExitCode, Stdio},
};
use which::which;

#[derive(Debug, Deserialize)]
struct Config {
    remote: RemoteConfig,
}

#[derive(Debug, Deserialize)]
struct RemoteConfig {
    host: String,
    #[serde(default = "default_ssh_port")]
    ssh_port: u16,
    #[serde(default = "default_build_dir")]
    build_dir: String,
    #[serde(default)]
    env_file: Option<String>,
    #[serde(default = "default_exclude")]
    exclude: Vec<String>,
}

fn default_ssh_port() -> u16 { 22 }
fn default_build_dir() -> String { "~/abrasive-builds".to_string() }
fn default_exclude() -> Vec<String> { vec!["target".to_string(), ".git".to_string()] }

/// Walk up from `start` looking for abrasive.toml.
/// Returns (path_to_toml, project_root_dir).
fn find_config(start: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join("abrasive.toml");
        if candidate.is_file() {
            return Some((candidate, dir));
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Compute the relative path from project_root to cwd.
/// e.g. if root is /home/user/myproject and cwd is /home/user/myproject/crates/foo
/// returns Some("crates/foo").
fn relative_subdir(project_root: &Path, cwd: &Path) -> Option<PathBuf> {
    cwd.strip_prefix(project_root).ok().and_then(|rel| {
        if rel.as_os_str().is_empty() {
            None
        } else {
            Some(rel.to_path_buf())
        }
    })
}

fn run_local(args: &[String]) -> ExitCode {
    debug!("No abrasive.toml found, running cargo locally");
    let status = Command::new("cargo")
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .stdin(Stdio::inherit())
        .status();

    match status {
        Ok(s) => ExitCode::from(s.code().unwrap_or(1) as u8),
        Err(e) => {
            error!("Failed to run cargo: {e}");
            ExitCode::from(1)
        }
    }
}

fn run_remote(config: &Config, project_root: &Path, subdir: Option<PathBuf>, args: &[String]) -> ExitCode {
    let remote = &config.remote;

    // Check rsync is available
    which("rsync").unwrap_or_else(|e| {
        error!("rsync not found in $PATH, please install it ({e})");
        exit(1);
    });

    // Build a stable remote path from the project root
    let project_name = project_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());

    let build_path = format!("{}/{}", remote.build_dir, project_name);

    // 1. rsync project to remote
    info!("Syncing to {}:{}", remote.host, build_path);
    let mut rsync = Command::new("rsync");
    rsync
        .arg("-a")
        .arg("--delete")
        .arg("--compress")
        .arg("-e")
        .arg(format!("ssh -p {}", remote.ssh_port))
        .arg("--info=progress2");

    for pattern in &remote.exclude {
        rsync.arg("--exclude").arg(pattern);
    }

    let rsync_path_arg = format!("mkdir -p {build_path} && rsync");
    rsync
        .arg("--rsync-path")
        .arg(rsync_path_arg)
        .arg(format!("{}/", project_root.display()))
        .arg(format!("{}:{}", remote.host, build_path))
        .env("LC_ALL", "C.UTF-8")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .stdin(Stdio::inherit());

    let rsync_result = rsync.status();
    match rsync_result {
        Ok(s) if !s.success() => {
            error!("rsync failed with exit code: {s}");
            return ExitCode::from(1);
        }
        Err(e) => {
            error!("Failed to run rsync: {e}");
            return ExitCode::from(1);
        }
        _ => {}
    }

    // 2. Build the remote command
    let cd_target = match &subdir {
        Some(rel) => format!("{build_path}/{}", rel.display()),
        None => build_path.clone(),
    };

    let mut remote_cmd = String::new();

    if let Some(env_file) = &remote.env_file {
        remote_cmd.push_str(&format!("source {env_file} && "));
    }

    remote_cmd.push_str(&format!("cd {} && cargo {}", cd_target, args.join(" ")));

    info!("Running: cargo {} (on {})", args.join(" "), remote.host);

    let ssh_status = Command::new("ssh")
        .env("LC_ALL", "C.UTF-8")
        .args(["-p", &remote.ssh_port.to_string()])
        .arg("-t")
        .arg(&remote.host)
        .arg(&remote_cmd)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .stdin(Stdio::inherit())
        .status();

    match ssh_status {
        Ok(s) => ExitCode::from(s.code().unwrap_or(1) as u8),
        Err(e) => {
            error!("Failed to ssh to {}: {e}", remote.host);
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    let all_args: Vec<String> = env::args().collect();

    // If invoked as "abrasive --version" or "abrasive --help", handle it
    if all_args.len() == 2 && (all_args[1] == "--version" || all_args[1] == "-V") {
        println!("abrasive {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if all_args.len() == 2 && (all_args[1] == "--help" || all_args[1] == "-h") {
        println!("abrasive {} — transparent cargo wrapper with remote builds", env!("CARGO_PKG_VERSION"));
        println!();
        println!("USAGE:");
        println!("    abrasive <cargo-args>...");
        println!("    cargo <cargo-args>...     (if aliased: alias cargo=abrasive)");
        println!();
        println!("If abrasive.toml exists in the current or any parent directory,");
        println!("the cargo command is forwarded to the configured remote build server.");
        println!("Otherwise, cargo is invoked locally.");
        println!();
        println!("SETUP:");
        println!("    abrasive setup            Create abrasive.toml interactively");
        return ExitCode::SUCCESS;
    }

    // Everything after argv[0] is forwarded to cargo
    let cargo_args: Vec<String> = all_args[1..].to_vec();

    if cargo_args.is_empty() {
        // No args — just run cargo with no args (shows cargo help)
        return run_local(&cargo_args);
    }

    // Handle "abrasive setup" / "cargo setup"
    if cargo_args[0] == "setup" {
        return run_setup();
    }

    let cwd = env::current_dir().unwrap_or_else(|e| {
        error!("Cannot determine current directory: {e}");
        exit(1);
    });

    match find_config(&cwd) {
        Some((config_path, project_root)) => {
            debug!("Found config at {}", config_path.display());
            let config_str = fs::read_to_string(&config_path).unwrap_or_else(|e| {
                error!("Failed to read {}: {e}", config_path.display());
                exit(1);
            });
            let config: Config = toml::from_str(&config_str).unwrap_or_else(|e| {
                error!("Failed to parse {}: {e}", config_path.display());
                exit(1);
            });
            let subdir = relative_subdir(&project_root, &cwd);
            run_remote(&config, &project_root, subdir, &cargo_args)
        }
        None => run_local(&cargo_args),
    }
}

fn run_setup() -> ExitCode {
    let cwd = env::current_dir().unwrap_or_else(|e| {
        error!("Cannot determine current directory: {e}");
        exit(1);
    });

    let config_path = cwd.join("abrasive.toml");
    if config_path.exists() {
        println!("abrasive.toml already exists in this directory.");
        return ExitCode::from(1);
    }

    let default_config = r#"[remote]
host = "your-build-server"
# ssh_port = 22
# build_dir = "~/abrasive-builds"
# env_file = "~/.profile"
# exclude = ["target", ".git"]
"#;

    fs::write(&config_path, default_config).unwrap_or_else(|e| {
        error!("Failed to write abrasive.toml: {e}");
        exit(1);
    });

    println!("Created abrasive.toml — edit it to set your remote build server.");
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn find_config_in_current_dir() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("abrasive.toml"), "[remote]\nhost = \"test\"").unwrap();
        let (path, root) = find_config(dir.path()).unwrap();
        assert_eq!(path, dir.path().join("abrasive.toml"));
        assert_eq!(root, dir.path());
    }

    #[test]
    fn find_config_in_parent() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("abrasive.toml"), "[remote]\nhost = \"test\"").unwrap();
        let child = dir.path().join("sub").join("deep");
        fs::create_dir_all(&child).unwrap();
        let (path, root) = find_config(&child).unwrap();
        assert_eq!(path, dir.path().join("abrasive.toml"));
        assert_eq!(root, dir.path());
    }

    #[test]
    fn find_config_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_config(dir.path()).is_none());
    }

    #[test]
    fn relative_subdir_works() {
        let root = Path::new("/home/user/project");
        let cwd = Path::new("/home/user/project/crates/foo");
        assert_eq!(relative_subdir(root, cwd), Some(PathBuf::from("crates/foo")));
    }

    #[test]
    fn relative_subdir_same_dir() {
        let root = Path::new("/home/user/project");
        assert_eq!(relative_subdir(root, root), None);
    }
}
