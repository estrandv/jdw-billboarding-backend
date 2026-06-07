use std::env;
use jdw_billboarding_backend::{parse_billboard_file, dump_queue_update};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <file.bbd>", args[0]);
        std::process::exit(1);
    }
    let bb = parse_billboard_file(&args[1])
        .unwrap_or_else(|e| { eprintln!("Parse error: {}", e); std::process::exit(1); });
    let lines = dump_queue_update(&bb);
    for line in lines {
        println!("{}", line);
    }
}
