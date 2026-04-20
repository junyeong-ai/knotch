//! `knotch log` — dump a unit's event log.

use anyhow::{Context as _, anyhow};
use clap::Args as ClapArgs;
use serde_json::Value;

use crate::{cmd::OutputMode, config::Config};

/// `knotch log` arguments.
#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// Unit to dump.
    pub unit: String,
    /// Cap on the number of events to print (most-recent first when
    /// `--tail` is present).
    #[arg(long)]
    pub limit: Option<usize>,
    /// Print only the most-recent `limit` events.
    #[arg(long)]
    pub tail: bool,
}

/// Run the log command.
///
/// # Errors
/// Returns an error if the unit's log file cannot be read or parsed.
pub(crate) async fn run(config: &Config, out: OutputMode, args: Args) -> anyhow::Result<()> {
    let path = config.unit_log(&args.unit);
    let lines = super::read_log_lines(&path).await?;
    if lines.is_empty() {
        return Err(anyhow!(
            "no events recorded for unit {:?} (expected {})",
            args.unit,
            path.display()
        ));
    }

    let mut parsed: Vec<Value> = Vec::with_capacity(lines.len());
    for (idx, line) in lines.iter().enumerate() {
        let value = serde_json::from_str::<Value>(line)
            .with_context(|| format!("parse line {} of {}", idx + 1, path.display()))?;
        parsed.push(value);
    }

    // Filter out the `__header__` sentinel — it's metadata, not an event.
    let mut events: Vec<Value> = parsed
        .into_iter()
        .filter(|v| v.get("kind").and_then(Value::as_str) != Some("__header__"))
        .collect();

    if args.tail {
        if let Some(limit) = args.limit {
            if events.len() > limit {
                events = events.split_off(events.len() - limit);
            }
        }
    } else if let Some(limit) = args.limit {
        events.truncate(limit);
    }

    match out {
        OutputMode::Human => {
            for (idx, evt) in events.iter().enumerate() {
                print_human_event(idx + 1, evt);
            }
            println!("({} event(s))", events.len());
        }
        OutputMode::Json => {
            let value = serde_json::Value::Array(events);
            println!("{value}");
        }
    }
    Ok(())
}

fn print_human_event(index: usize, value: &Value) {
    let kind = value.get("body")
        .and_then(|b| b.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let at = value.get("at").and_then(Value::as_str).unwrap_or("-");
    let id = value.get("id").and_then(Value::as_str).unwrap_or("-");
    println!("#{index:<4} {at}  {kind:<22} id={id}");
}
