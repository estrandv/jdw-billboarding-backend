use std::env;
use jdw_billboarding_backend::{
    parse_billboard_file, dump_queue_update, dump_setup, dump_commands, load_synthdefs,
};
use jdw_billboarding_backend::config::JdwConfig;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} [--phase setup|commands|play|all] <file.bbd>", args[0]);
        std::process::exit(1);
    }

    let mut phase = "all";
    let path;
    if args[1] == "--phase" {
        if args.len() < 4 {
            eprintln!("Expected: --phase <name> <file.bbd>");
            std::process::exit(1);
        }
        phase = &args[2];
        path = &args[3];
    } else {
        path = &args[1];
    }

    let cfg = JdwConfig::load(None);

    if phase == "setup" || phase == "all" {
        let defs = load_synthdefs(
            cfg.synthdefs_scd_path.as_deref(),
            cfg.template_synths_path.as_deref(),
            cfg.bbd_root.as_deref(),
        );
        for line in dump_setup(&defs) {
            println!("{}", line);
        }
    }

    if phase == "play" || phase == "all" {
        let bb = parse_billboard_file(path)
            .unwrap_or_else(|e| { eprintln!("Parse error: {}", e); std::process::exit(1); });
        for line in dump_queue_update(&bb) {
            println!("{}", line);
        }
    }

    if phase == "commands" || phase == "all" {
        let bb = parse_billboard_file(path)
            .unwrap_or_else(|e| { eprintln!("Parse error: {}", e); std::process::exit(1); });
        for line in dump_commands(&bb) {
            println!("{}", line);
        }
    }
}
