//! `codeisland` — hook installer, doctor, diagnostics and manual event sender.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use codeisland_agents::ToolStatus;
use codeisland_domain::{AgentKind, AppSettings};
use serde_json::{Value, json};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(
    name = "codeisland",
    version,
    about = "CodeIsland hook installer, doctor, diagnostics and event sender"
)]
struct Cli {
    /// Limit a hook operation to one agent tag (e.g. claude, codex).
    #[arg(long, global = true)]
    agent: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect or change agent hook installations.
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },
    /// Print hook status, runtime state path and version.
    Doctor,
    /// Write a JSON diagnostics bundle.
    Diagnostics {
        #[arg(long, value_name = "PATH")]
        out: PathBuf,
    },
    /// Read an agent event JSON document and hand it to the hook client.
    Send {
        #[arg(long)]
        source: String,
        #[arg(long)]
        event: String,
        /// Read from this file instead of stdin.
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
    },
    /// Print the version.
    Version,
}

#[derive(Subcommand)]
enum HooksAction {
    Status,
    Install,
    Repair,
    Uninstall,
}

fn main() {
    let cli = Cli::parse();
    if let Err(error) = run(cli) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let Cli { agent, command } = cli;
    match command {
        Command::Hooks { action } => run_hooks(action, agent),
        Command::Doctor => run_doctor(),
        Command::Diagnostics { out } => run_diagnostics(&out),
        Command::Send {
            source,
            event,
            file,
        } => run_send(&source, &event, file.as_deref()),
        Command::Version => {
            println!("{VERSION}");
            Ok(())
        }
    }
}

fn run_hooks(action: HooksAction, agent: Option<String>) -> Result<()> {
    match action {
        HooksAction::Status => {
            let statuses = match resolve_agent(agent.as_deref())? {
                Some(agent) => vec![codeisland_agents::detect(agent)],
                None => codeisland_agents::detect_all(),
            };
            print_table(&statuses);
            Ok(())
        }
        HooksAction::Install => apply_to_agents(agent.as_deref(), codeisland_agents::install),
        HooksAction::Repair => apply_to_agents(agent.as_deref(), codeisland_agents::repair),
        HooksAction::Uninstall => {
            apply_to_agents(agent.as_deref(), codeisland_agents::uninstall)
        }
    }
}

fn apply_to_agents(
    tag: Option<&str>,
    operation: fn(AgentKind) -> Result<ToolStatus, codeisland_agents::AgentError>,
) -> Result<()> {
    let agents = match resolve_agent(tag)? {
        Some(agent) => vec![agent],
        None => codeisland_agents::definitions()
            .iter()
            .map(|definition| definition.agent)
            .collect(),
    };
    let mut statuses = Vec::new();
    let mut failures = 0usize;
    for agent in agents {
        match operation(agent) {
            Ok(status) => statuses.push(status),
            Err(error) => {
                failures += 1;
                eprintln!("{}: {error}", agent.display_name());
            }
        }
    }
    print_table(&statuses);
    if failures > 0 {
        bail!("{failures} agent(s) failed");
    }
    Ok(())
}

fn resolve_agent(tag: Option<&str>) -> Result<Option<AgentKind>> {
    match tag {
        None => Ok(None),
        Some(tag) => {
            let agent = AgentKind::from_tag(tag);
            if agent == AgentKind::Unknown && !tag.eq_ignore_ascii_case("unknown") {
                bail!("unknown agent tag: {tag}");
            }
            Ok(Some(agent))
        }
    }
}

fn print_table(statuses: &[ToolStatus]) {
    if statuses.is_empty() {
        println!("No agents found.");
        return;
    }
    let headers = ["Agent", "Installed", "Hook", "Healthy", "Config", "Problem"];
    let mut rows: Vec<[String; 6]> = Vec::with_capacity(statuses.len());
    for status in statuses {
        rows.push([
            format!("{} ({})", status.display_name, status.agent.tag()),
            yes_no(status.installed),
            yes_no(status.hook_installed),
            yes_no(status.healthy),
            status.config_path.clone().unwrap_or_else(|| "-".into()),
            status.problem.clone().unwrap_or_else(|| "-".into()),
        ]);
    }
    let mut widths = headers.map(str::len);
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }
    let line = |cells: [&str; 6]| {
        cells
            .iter()
            .enumerate()
            .map(|(index, cell)| format!("{cell:<width$}", width = widths[index]))
            .collect::<Vec<_>>()
            .join("  ")
    };
    println!("{}", line(headers));
    println!("{}", "-".repeat(widths.iter().sum::<usize>() + 10));
    for row in &rows {
        let cells = [
            row[0].as_str(),
            row[1].as_str(),
            row[2].as_str(),
            row[3].as_str(),
            row[4].as_str(),
            row[5].as_str(),
        ];
        println!("{}", line(cells));
    }
}

fn yes_no(value: bool) -> String {
    if value { "yes".into() } else { "no".into() }
}

fn run_doctor() -> Result<()> {
    println!("CodeIsland {VERSION}");
    println!("OS: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    println!("State: {}", state_path().display());
    println!(
        "Hook command: {}",
        codeisland_agents::hook_command(AgentKind::Claude)
    );
    println!();
    print_table(&codeisland_agents::detect_all());
    Ok(())
}

fn run_diagnostics(out: &Path) -> Result<()> {
    let hooks: Vec<Value> = codeisland_agents::detect_all()
        .iter()
        .map(hook_status_json)
        .collect();
    let settings = read_settings();
    let document = json!({
        "version": VERSION,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "generated_at": chrono::Utc::now(),
        "state_path": state_path().display().to_string(),
        "hook_command": codeisland_agents::hook_command(AgentKind::Claude),
        "settings": settings,
        "hooks": hooks,
    });
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
    }
    let text = serde_json::to_string_pretty(&document)?;
    std::fs::write(out, text).with_context(|| format!("write {}", out.display()))?;
    println!("Wrote diagnostics to {}", out.display());
    Ok(())
}

fn hook_status_json(status: &ToolStatus) -> Value {
    json!({
        "agent": status.agent.tag(),
        "display_name": status.display_name,
        "installed": status.installed,
        "hook_installed": status.hook_installed,
        "healthy": status.healthy,
        "config_path": status.config_path.clone(),
        "command": status.command.clone(),
        "problem": status.problem.clone(),
    })
}

fn run_send(source: &str, event: &str, file: Option<&Path>) -> Result<()> {
    let raw = match file {
        Some(path) => {
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?
        }
        None => {
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .context("read stdin")?;
            buffer
        }
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("no JSON event supplied");
    }
    let mut value: Value = serde_json::from_str(trimmed).context("input is not valid JSON")?;
    // `--event` is authoritative when the document omits the hook event name.
    if let Value::Object(map) = &mut value {
        map.entry("hook_event_name")
            .or_insert_with(|| Value::String(event.to_owned()));
    }
    let payload = serde_json::to_string(&value)?;

    let program = resolve_hook_program();
    let mut child = ProcessCommand::new(&program)
        .args(["send", "--source", source, "--event", event, "--stdin"])
        .env("CODEISLAND_SOURCE", source)
        .env("CODEISLAND_EVENT", event)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("launch {}", program.display()))?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin
            .write_all(payload.as_bytes())
            .context("write hook stdin")?;
        stdin.write_all(b"\n").ok();
    }
    drop(child.stdin.take());

    let output = child.wait_with_output().context("wait for hook")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        print!("{stdout}");
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }
    if !output.status.success() {
        bail!(
            "hook exited with status {}",
            output.status.code().unwrap_or(-1)
        );
    }
    Ok(())
}

/// Resolves the `codeisland-hook` executable the same way the agent installer
/// does: `CODEISLAND_HOOK`, else beside this binary, else `PATH`.
fn resolve_hook_program() -> PathBuf {
    if let Some(raw) = std::env::var_os("CODEISLAND_HOOK") {
        if !raw.is_empty() {
            return PathBuf::from(raw);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(directory) = exe.parent() {
            for name in ["codeisland-hook.exe", "codeisland-hook"] {
                let candidate = directory.join(name);
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from("codeisland-hook")
}

fn state_path() -> PathBuf {
    codeisland_runtime::persistence::state_path()
}

fn read_settings() -> AppSettings {
    codeisland_runtime::persistence::load_settings()
        .map(AppSettings::validate)
        .unwrap_or_default()
}
