use cocoon_runtime::Task;
use std::{fs::File, io::Read};

fn main() {
    if let Err(error) = run() {
        eprintln!("cocoon: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || !matches!(args[0].as_str(), "plan" | "plan-spool") {
        return Err(
            "usage: cocoon plan|plan-spool TASK.json (validation only; no execution)".into(),
        );
    }
    let mut bytes = Vec::new();
    File::open(&args[1])?.take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err("task exceeds 64 KiB".into());
    }
    let task = if args[0] == "plan-spool" {
        let task: cocoon_runtime::spool::SpoolTask = serde_json::from_slice(&bytes)?;
        task.validate()?;
        serde_json::to_value(task)?
    } else {
        let task: Task = serde_json::from_slice(&bytes)?;
        task.validate()?;
        serde_json::to_value(task)?
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "cocoon-plan-v1", "task": task,
            "execution_authorized": false, "backend_qualified": false,
            "execution_available": false, "native_fallback": false,
            "note": "Contract validation only. No VM or process was started."
        }))?
    );
    Ok(())
}
