#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--serve") => {
            if let Err(error) = gatetree_ops_agent_core::run_server() {
                eprintln!("GateTree-Ops Agent failed: {error}");
                std::process::exit(1);
            }
        }
        Some("--status") => match gatetree_ops_agent_core::status() {
            Ok(status) => println!(
                "running pid={} version={}",
                status.pid.unwrap_or_default(),
                status.version.as_deref().unwrap_or("unknown")
            ),
            Err(_) => {
                println!("stopped");
                std::process::exit(3);
            }
        },
        Some("--start") => match gatetree_ops_agent_core::start() {
            Ok(status) => println!(
                "running pid={} version={}",
                status.pid.unwrap_or_default(),
                status.version.as_deref().unwrap_or("unknown")
            ),
            Err(error) => {
                eprintln!("GateTree-Ops Agent start failed: {error}");
                std::process::exit(1);
            }
        },
        Some("--kill") => {
            if let Err(error) = gatetree_ops_agent_core::kill() {
                eprintln!("GateTree-Ops Agent kill failed: {error}");
                std::process::exit(1);
            }
            println!("stopped");
        }
        Some("--stop") => {
            if let Err(error) = gatetree_ops_agent_core::stop() {
                eprintln!("GateTree-Ops Agent stop failed: {error}");
                std::process::exit(1);
            }
            println!("stopped");
        }
        _ => {
            eprintln!("Usage: gatetree-ops-agent --serve|--start|--status|--stop|--kill");
            std::process::exit(2);
        }
    }
}
