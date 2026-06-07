use std::fs;
use std::process;

use jdw_billboarding_backend::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: jdw <command> [args..]");
        eprintln!();
        eprintln!("Commands:");
        eprintln!("  stop                  Send stop signals to router");
        eprintln!("  play <file.bbd>       Full pipeline: setup + queue + commands");
        eprintln!("  setup <file.bbd>      Send synth setup messages");
        eprintln!("  update <file.bbd>     Send sequencer queue update");
        eprintln!("  cmd <file.bbd>        Send billboard commands");
        process::exit(1);
    }

    let config = osc::OscConfig::default();

    match args[1].as_str() {
        "stop" | "terminate" => {
            if let Err(e) = osc::send_stop(&config) {
                eprintln!("Error sending stop: {}", e);
                process::exit(1);
            }
            println!("Stop signals sent.");
        }
        "play" | "all" => {
            let billboard = parse_file(&args, 2);
            if let Err(e) = osc::send_full_billboard(&billboard, &config) {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
            println!("Billboard sent (setup + update + commands).");
        }
        "setup" => {
            let billboard = parse_file(&args, 2);
            if let Err(e) = osc::send_full_setup(&billboard, &config) {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
            println!("Setup messages sent.");
        }
        "update" => {
            let billboard = parse_file(&args, 2);
            if let Err(e) = osc::send_full_queue_update(&billboard, &config) {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
            println!("Queue update sent.");
        }
        "cmd" | "commands" => {
            let billboard = parse_file(&args, 2);
            if let Err(e) = osc::send_full_commands(&billboard, &config) {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
            println!("Commands sent.");
        }
        _ => {
            eprintln!("Unknown command: {}", args[1]);
            process::exit(1);
        }
    }
}

fn parse_file(args: &[String], index: usize) -> full::Billboard {
    let path = args.get(index).unwrap_or_else(|| {
        eprintln!("Usage: jdw {} <file.bbd>", args[1]);
        process::exit(1);
    });
    let source = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {}", path, e);
        process::exit(1);
    });
    full::parse(&source)
}
