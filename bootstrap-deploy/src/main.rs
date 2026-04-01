use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

const SCAN_DURATION: Duration = Duration::from_secs(5);
const REMOTE_DIR: &str = "tunnels";
const REMOTE_BINARY: &str = "tunnels/tunnel-bootstrap";
const PLIST_FILENAME: &str = "local.tunnelbootstrap.plist";

const PLIST_TEMPLATE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>local.tunnelbootstrap</string>
    <key>ProgramArguments</key>
    <array>
        <string>__HOME__/tunnels/tunnel-bootstrap</string>
    </array>
    <key>KeepAlive</key>
    <true/>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardErrorPath</key>
    <string>__HOME__/tunnels/bootstrap.log</string>
    <key>StandardOutPath</key>
    <string>__HOME__/tunnels/bootstrap.log</string>
    <key>WorkingDirectory</key>
    <string>__HOME__/tunnels</string>
</dict>
</plist>
"#;

struct SshTarget {
    instance_name: String,
    hostname: String,
    ip: String,
    port: u16,
}

fn discover_ssh_targets() -> Result<Vec<SshTarget>> {
    let daemon = ServiceDaemon::new().context("Failed to create mDNS daemon")?;
    let receiver = daemon
        .browse("_ssh._tcp.local.")
        .context("Failed to browse for SSH services")?;

    let mut targets: HashMap<String, SshTarget> = HashMap::new();

    println!("Scanning for SSH-enabled machines...\n");

    let deadline = std::time::Instant::now() + SCAN_DURATION;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match receiver.recv_timeout(remaining) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let instance_name = info.get_fullname().to_string();
                let display_name = instance_name
                    .strip_suffix("._ssh._tcp.local.")
                    .unwrap_or(&instance_name)
                    .to_string();

                let ip = info
                    .get_addresses_v4()
                    .into_iter()
                    .next()
                    .map(|a| a.to_string())
                    .unwrap_or_default();

                let hostname = info
                    .get_hostname()
                    .strip_suffix('.')
                    .unwrap_or(info.get_hostname())
                    .to_string();

                targets.insert(
                    display_name.clone(),
                    SshTarget {
                        instance_name: display_name,
                        hostname,
                        ip,
                        port: info.get_port(),
                    },
                );
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    let _ = daemon.shutdown();

    let mut result: Vec<SshTarget> = targets.into_values().collect();
    result.sort_by(|a, b| a.instance_name.cmp(&b.instance_name));
    Ok(result)
}

/// Open an authenticated SSH session to the target.
fn open_session(target: &SshTarget, username: &str, password: &str) -> Result<ssh2::Session> {
    let addr = if target.ip.is_empty() {
        format!("{}:{}", target.hostname, target.port)
    } else {
        format!("{}:{}", target.ip, target.port)
    };

    let tcp = TcpStream::connect_timeout(&addr.parse()?, Duration::from_secs(5))
        .with_context(|| format!("TCP connect to {addr}"))?;

    let mut session = ssh2::Session::new().context("Failed to create SSH session")?;
    session.set_tcp_stream(tcp);
    session.handshake().context("SSH handshake failed")?;
    session
        .userauth_password(username, password)
        .context("Authentication failed")?;

    Ok(session)
}

/// Run a command over SSH and return its stdout.
fn ssh_exec(session: &ssh2::Session, command: &str) -> Result<String> {
    let mut channel = session
        .channel_session()
        .context("Failed to open channel")?;
    channel.exec(command)?;
    let mut output = String::new();
    channel.read_to_string(&mut output)?;
    let mut stderr = String::new();
    channel.stderr().read_to_string(&mut stderr)?;
    channel.wait_close()?;
    let exit = channel.exit_status()?;
    if exit != 0 {
        anyhow::bail!("Command `{command}` exited {exit}: {stderr}");
    }
    Ok(output)
}

/// Send a local file to a remote path via SCP.
fn scp_send(
    session: &ssh2::Session,
    local_path: &Path,
    remote_path: &str,
    mode: i32,
) -> Result<()> {
    let data = std::fs::read(local_path)
        .with_context(|| format!("Read local file {}", local_path.display()))?;
    let mut channel = session
        .scp_send(Path::new(remote_path), mode, data.len() as u64, None)
        .with_context(|| format!("SCP open {remote_path}"))?;
    channel.write_all(&data)?;
    channel.send_eof()?;
    channel.wait_eof()?;
    channel.wait_close()?;
    Ok(())
}

/// Send raw bytes to a remote path via SCP.
fn scp_send_bytes(
    session: &ssh2::Session,
    data: &[u8],
    remote_path: &str,
    mode: i32,
) -> Result<()> {
    let mut channel = session
        .scp_send(Path::new(remote_path), mode, data.len() as u64, None)
        .with_context(|| format!("SCP open {remote_path}"))?;
    channel.write_all(data)?;
    channel.send_eof()?;
    channel.wait_eof()?;
    channel.wait_close()?;
    Ok(())
}

/// Deploy the bootstrapper to a remote machine.
fn deploy(
    target: &SshTarget,
    username: &str,
    password: &str,
    binary_path: &Path,
) -> Result<()> {
    let session = open_session(target, username, password)?;

    // Resolve the remote user's home directory.
    let home = ssh_exec(&session, "echo $HOME")?.trim().to_string();
    let remote_dir = format!("{home}/{REMOTE_DIR}");
    let remote_binary = format!("{home}/{REMOTE_BINARY}");

    // Create ~/tunnels/.
    println!("  Creating {remote_dir}...");
    ssh_exec(&session, &format!("mkdir -p {remote_dir}"))?;

    // Unload existing daemon (ignore failure — may not be loaded yet).
    println!("  Stopping existing daemon (if any)...");
    let _ = ssh_exec(
        &session,
        "launchctl bootout gui/$(id -u)/local.tunnelbootstrap 2>/dev/null || true",
    );

    // Upload the binary.
    println!(
        "  Uploading binary ({:.1} MB)...",
        binary_path.metadata()?.len() as f64 / 1_000_000.0
    );
    scp_send(&session, binary_path, &remote_binary, 0o755)?;

    // Resolve the embedded plist template with the remote home directory.
    let plist_content = PLIST_TEMPLATE.replace("__HOME__", &home);

    // Write the resolved plist to a temp file, then SCP it.
    let tmp_plist = "/tmp/local.tunnelbootstrap.plist";
    println!("  Uploading plist...");
    scp_send_bytes(&session, plist_content.as_bytes(), tmp_plist, 0o644)?;
    ssh_exec(
        &session,
        &format!(
            "mkdir -p ~/Library/LaunchAgents && mv {tmp_plist} ~/Library/LaunchAgents/{PLIST_FILENAME}"
        ),
    )?;

    // Load the daemon.
    println!("  Loading daemon...");
    ssh_exec(
        &session,
        "launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/local.tunnelbootstrap.plist",
    )?;

    // Verify it's running.
    println!("  Verifying...");
    let status = ssh_exec(
        &session,
        "launchctl print gui/$(id -u)/local.tunnelbootstrap 2>&1 | head -5",
    )?;
    println!(
        "  {}",
        status.trim().lines().next().unwrap_or("(no output)")
    );

    Ok(())
}

fn prompt_line(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

fn main() -> Result<()> {
    let targets = discover_ssh_targets()?;

    if targets.is_empty() {
        println!("No machines with SSH (Remote Login) found on the network.");
        println!("Enable it on target Macs: System Settings > General > Sharing > Remote Login");
        return Ok(());
    }

    println!("Found {} machine(s):", targets.len());
    for (i, t) in targets.iter().enumerate() {
        println!("  {}. {:<30} {}:{}", i + 1, t.instance_name, t.ip, t.port);
    }
    println!();

    let selection = prompt_line(&format!("Select a machine (1-{}): ", targets.len()))?;
    let index: usize = selection
        .parse::<usize>()
        .context("Expected a number")?
        .checked_sub(1)
        .context("Selection out of range")?;
    let target = targets.get(index).context("Selection out of range")?;

    println!();
    let username = prompt_line(&format!("Username for {}: ", target.instance_name))?;
    let password = rpassword::prompt_password(format!(
        "Password for {username}@{}: ",
        target.instance_name
    ))?;
    println!();

    let default_binary_path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("tunnel-bootstrap")))
        .unwrap_or_else(|| Path::new("tunnel-bootstrap").to_path_buf());
    let prompt_msg = format!(
        "Path to tunnel-bootstrap binary [{}]: ",
        default_binary_path.display()
    );
    let binary_path_str = prompt_line(&prompt_msg)?;
    let binary_path = if binary_path_str.is_empty() {
        default_binary_path.as_path()
    } else {
        Path::new(&binary_path_str)
    };
    anyhow::ensure!(
        binary_path.exists(),
        "Binary not found: {}",
        binary_path.display()
    );

    println!("Deploying to {}...\n", target.instance_name);
    deploy(target, &username, &password, binary_path)?;
    println!("\nDone. Bootstrapper should appear in the console shortly.");

    Ok(())
}
