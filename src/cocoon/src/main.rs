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
    if args.first().is_some_and(|s| s == "host") {
        return host(&args[1..]);
    }
    if args.len() != 2 || args[0] != "plan" {
        return Err("usage: cocoon plan TASK.json (validation only; no execution)".into());
    }
    let mut bytes = Vec::new();
    File::open(&args[1])?.take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err("task exceeds 64 KiB".into());
    }
    let task: Task = serde_json::from_slice(&bytes)?;
    task.validate()?;
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

fn host(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use cocoon_host::{FinishPolicy, Manager};
    use std::path::Path;
    use uuid::Uuid;
    if args.len() < 2 {
        return Err("usage: cocoon host ABSOLUTE_STATE_DIR COMMAND ...".into());
    }
    let manager = Manager::open(Path::new(&args[0]))?;
    let result = match args[1..].iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["image", "import", kernel, initrd, disk, provenance] => serde_json::json!({"image": manager.import_image(Path::new(kernel), Path::new(initrd), Path::new(disk), provenance)?}),
        ["image", "show", id] => manager.image(Uuid::parse_str(id)?.into())?,
        ["vm", "create", id, image, owner, policy] => {
            let policy = match *policy { "retain" => FinishPolicy::Retain, "rollback" => FinishPolicy::Rollback, "branch" => FinishPolicy::Branch, _ => return Err("invalid finish policy".into()) };
            serde_json::to_value(manager.create(Uuid::parse_str(id)?.into(), Uuid::parse_str(image)?.into(), owner, policy)?)?
        }
        ["vm", "list"] => serde_json::to_value(manager.list()?)?,
        ["vm", "show", id] => serde_json::to_value(manager.get(Uuid::parse_str(id)?.into())?)?,
        ["experiment", "persistence", id, runner, mode, payload] => {
            let result = manager.persistence_boot(Uuid::parse_str(id)?.into(), Path::new(runner), mode, payload)?;
            if result["passed"] != true { println!("{}", serde_json::to_string_pretty(&result)?); return Err("persistence acceptance failed; inspect original boot before further action".into()); }
            result
        }
        ["experiment", "project", id, runner] => {
            let result = manager.project_boot(Uuid::parse_str(id)?.into(), Path::new(runner))?;
            if result["passed"] != true { println!("{}", serde_json::to_string_pretty(&result)?); return Err("project acceptance failed; reconcile original boot before further action".into()); }
            result
        }
        ["vm", "start", id] => { manager.start(Uuid::parse_str(id)?.into())?; serde_json::Value::Null }
        ["operations"] => serde_json::to_value(manager.operations()?)?,
        _ => return Err("unknown host command; supported: image import/show, vm create/list/show/start, operations".into()),
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
