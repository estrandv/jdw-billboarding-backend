use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// A loaded SynthDef ready to send via `/create_synthdef`.
#[derive(Debug, Clone)]
pub struct SynthDefMessage {
    pub name: String,
    pub content: String,
}

/// Parse comma-separated args in `keyVALUE` format.
///
/// `arg1=1.0,arg2=2.0` is NOT the input format — the template syntax
/// uses `keyVALUE` without `=` (the `=` is added during compilation).
/// Python: `re.findall("[a-zA-z]+", atom)[0]` extracts leading letters,
/// then `atom.replace(letter_part, "")` removes them to get the value.
///
/// `freq440,amp1,gate1` → `[("freq", "440"), ("amp", "1"), ("gate", "1")]`
fn parse_args(arg_string: &str) -> Vec<(String, String)> {
    arg_string
        .split(',')
        .filter_map(|atom| {
            let atom = atom.trim();
            if atom.is_empty() {
                return None;
            }
            // Extract leading consecutive letters (the key)
            let letter_end = atom.find(|c: char| !c.is_alphabetic())?;
            if letter_end == 0 {
                return None;
            }
            let key = atom[..letter_end].to_string();
            // Remove key from atom to get value (like Python's replace)
            let value = atom[letter_end..].to_string();
            Some((key, value))
        })
        .collect()
}

/// Extract variable name from a line like `osc = SinOsc.ar(...)`.
fn find_variable(scd_call: &str) -> Option<String> {
    let trimmed = scd_call.trim();
    if let Some(eq_pos) = trimmed.find('=') {
        // Make sure it's a real declaration (left side is simple identifier),
        // not something like `!=` or `==`
        let lhs = trimmed[..eq_pos].trim();
        if lhs.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') && !lhs.is_empty()
        {
            return Some(lhs.to_string());
        }
    }
    None
}

/// Compile a single template-synth definition into a SynthDef string.
///
/// Template format:
/// ```text
/// synth_name
/// args: arg1=1.0,arg2=2.0
/// scd_line1
/// scd_line2
/// ...
/// ```
fn compile_template(definition: &str) -> Result<String, String> {
    let lines: Vec<&str> = definition.lines().filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return Err("empty definition".to_string());
    }

    let name = lines[0].trim();

    // Find the args line
    let argline_idx = lines.iter().position(|l| l.contains("args: "));
    let argline = argline_idx
        .and_then(|i| lines.get(i))
        .ok_or_else(|| format!("'{}' has no args line", name))?;

    let arg_str = argline
        .split("args: ")
        .nth(1)
        .ok_or_else(|| format!("invalid arg line in '{}': {}", name, argline))?;

    let mut args = parse_args(arg_str);

    // Deduplicate: when a section overrides a $args default (e.g. attT=0,attT=0.002),
    // keep the last occurrence. sclang errors on duplicate SynthDef args.
    let mut seen = HashSet::new();
    args.reverse();
    args.retain(|(k, _)| seen.insert(k.clone()));
    args.reverse();

    // Lines after the args line are the SCD body
    let scd_lines: Vec<&str> = lines[argline_idx.unwrap() + 1..]
        .iter()
        .map(|l| l.trim())
        .collect();

    // Find variables declared with `=` that aren't in args
    let arg_names: HashSet<&str> = args.iter().map(|(k, _)| k.as_str()).collect();
    let mut dec_args: Vec<String> = scd_lines
        .iter()
        .filter_map(|line| find_variable(line))
        .filter(|var| !arg_names.contains(var.as_str()))
        .collect();
    dec_args.sort();
    dec_args.dedup();

    let var_args: String = args
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join(",");

    let dec_args_str = dec_args.join(",");

    let scd_body = scd_lines.join("\n        ");

    let result = format!(
        "SynthDef.new(\"{name}\", {{|{var_args}|\n        var {dec_args};\n        {scd_body}\n    }})",
        name = name,
        var_args = var_args,
        dec_args = if dec_args_str.is_empty() {
            "nil".to_string()
        } else {
            dec_args_str
        },
        scd_body = scd_body,
    );

    Ok(result)
}

/// Load raw SynthDefs from an SCD file containing `SynthDef.new(...)` blocks.
///
/// Returns (name, full_synthdef_string) pairs.
/// Strip trailing `//` comment lines and whitespace from a SynthDef string.
///
/// Needed because `load_synthdefs_from_scd` splits on `SynthDef.new` and
/// everything up to the NEXT definition is included — including separator
/// comments like `// Effects below` that sit between definitions.
fn strip_trailing_sc_comments(scd: &str) -> String {
    let mut lines: Vec<&str> = scd.lines().collect();
    // Remove trailing blank lines and comment-only lines
    while let Some(last) = lines.last() {
        let trimmed = last.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            lines.pop();
        } else {
            break;
        }
    }
    lines.join("\n")
}

fn load_synthdefs_from_scd(path: &str) -> Result<Vec<SynthDefMessage>, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("read {}: {}", path, e))?;

    let mut result = Vec::new();

    for cut in content.split("SynthDef.new") {
        let cut = cut.trim();
        if cut.is_empty() {
            continue;
        }
        // Each cut starts with `("name", ...)` — reconstruct
        let full = format!("SynthDef.new{}", cut);
        // Strip trailing comments and whitespace that belong to the NEXT synthdef
        // e.g. "samplerALT {...}) // Effects below \n\n" → "samplerALT {...})"
        let clean = strip_trailing_sc_comments(&full);
        // Extract name from the first quoted string
        if let Some(start) = clean.find('"') {
            if let Some(end) = clean[start + 1..].find('"') {
                let name = clean[start + 1..start + 1 + end].to_string();
                result.push(SynthDefMessage {
                    name,
                    content: clean,
                });
            }
        }
    }

    Ok(result)
}

/// Load and compile template synths from a template file.
///
/// Template files use the `~` separator between synth definitions.
/// Macros are expanded before compilation.
fn load_synthdefs_from_templates(
    path: &str,
    common_macros_dir: Option<&str>,
) -> Result<Vec<SynthDefMessage>, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("read {}: {}", path, e))?;

    // Try loading common_macros.txt from the same songs directory
    let mut supplied_defs = Vec::new();
    if let Some(macros_dir) = common_macros_dir {
        let macros_path = Path::new(macros_dir).join("common_macros.txt");
        if macros_path.exists() {
            let common = fs::read_to_string(&macros_path)
                .map_err(|e| format!("read {:?}: {}", macros_path, e))?;
            supplied_defs = crate::macros::find_macro_defs(&common);
        }
    }

    // Also parse macro definitions from the template file itself
    // (template files define their own $macroname(arg1,arg2) helpers)
    let template_macros = crate::macros::find_macro_defs(&content);
    supplied_defs.extend(template_macros);

    // Expand macros in the template content
    let expanded = crate::macros::compile_macros(&content, &supplied_defs)?;

    let mut result = Vec::new();

    for section in expanded.split('~') {
        let section = section.trim();
        if section.is_empty() {
            continue;
        }
        match compile_template(section) {
            Ok(synthdef) => {
                // Extract name
                let name = section
                    .lines()
                    .next()
                    .unwrap_or("unknown")
                    .trim()
                    .to_string();
                result.push(SynthDefMessage {
                    name,
                    content: synthdef,
                });
            }
            Err(e) => {
                // Skip invalid definitions, but log
                log::warn!("failed to compile synthdef: {}", e);
            }
        }
    }

    Ok(result)
}

/// Load all known SynthDefs from config paths.
///
/// Reads from:
/// 1. `synthdefs_scd_path` — raw `SynthDef.new(...)` blocks
/// 2. `template_synths_path` — template synths compiled to full SynthDefs
pub fn load_synthdefs(
    synthdefs_scd_path: Option<&str>,
    template_synths_path: Option<&str>,
    common_macros_dir: Option<&str>,
) -> Vec<SynthDefMessage> {
    let mut all = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    // Load raw SCD synthdefs
    if let Some(path) = synthdefs_scd_path {
        if Path::new(path).exists() {
            match load_synthdefs_from_scd(path) {
                Ok(defs) => {
                    for def in defs {
                        if seen_names.insert(def.name.clone()) {
                            all.push(def);
                        }
                    }
                }
                Err(e) => log::warn!("failed to load synthdefs from '{}': {}", path, e),
            }
        }
    }

    // Load and compile template synths
    if let Some(path) = template_synths_path {
        if Path::new(path).exists() {
            match load_synthdefs_from_templates(path, common_macros_dir) {
                Ok(defs) => {
                    for def in defs {
                        if seen_names.insert(def.name.clone()) {
                            all.push(def);
                        }
                    }
                }
                Err(e) => log::warn!(
                    "failed to load template synths from '{}': {}",
                    path, e
                ),
            }
        }
    }

    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_args_basic() {
        let args = parse_args("freq440,amp1,gate1");
        assert_eq!(args.len(), 3);
        assert_eq!(args[0], ("freq".to_string(), "440".to_string()));
        assert_eq!(args[1], ("amp".to_string(), "1".to_string()));
        assert_eq!(args[2], ("gate".to_string(), "1".to_string()));
    }

    #[test]
    fn test_parse_args_with_decimal() {
        let args = parse_args("relT0.2,attT0.01");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], ("relT".to_string(), "0.2".to_string()));
        assert_eq!(args[1], ("attT".to_string(), "0.01".to_string()));
    }

    #[test]
    fn test_parse_args_example() {
        let args = parse_args("freq440,amp1,gate1,sus1,pan0,attT0,decT0,susL1,relT0.2,out0");
        assert_eq!(args.len(), 10);
        assert_eq!(args[0], ("freq".to_string(), "440".to_string()));
        assert_eq!(args[9], ("out".to_string(), "0".to_string()));
    }

    #[test]
    fn test_find_variable_simple() {
        assert_eq!(
            find_variable("osc = SinOsc.ar(freq)"),
            Some("osc".to_string())
        );
    }

    #[test]
    fn test_find_variable_no_declaration() {
        assert_eq!(find_variable("Out.ar(out, osc)"), None);
    }

    #[test]
    fn test_find_variable_amp_reassignment() {
        assert_eq!(
            find_variable("amp = amp * 3;"),
            Some("amp".to_string())
        );
    }

    #[test]
    fn test_compile_template_basic() {
        let input = "testSynth\nargs: freq440,amp1\nosc = SinOsc.ar(freq, mul: amp);\nOut.ar(out, osc)";
        let result = compile_template(input).unwrap();
        assert!(result.contains("SynthDef.new(\"testSynth\""));
        assert!(result.contains("|freq=440,amp=1|"));
        assert!(result.contains("var osc;"));
        assert!(result.contains("osc = SinOsc.ar(freq, mul: amp);"));
    }

    #[test]
    fn test_compile_template_with_dec_args() {
        let input = "wobble\nargs: freq440,amp1\namp = amp * 3;\nosc = Pulse.ar(freq, mul: amp / 4);\nOut.ar(out, osc)";
        let result = compile_template(input).unwrap();
        // amp is already an arg, not in dec_args; osc is a dec_arg
        assert!(result.contains("var osc;"));
        assert!(!result.contains("var amp,"));
    }

    #[test]
    fn test_compile_template_wobble_like() {
        // Simulates wobble after $args macro expansion
        let input = "\
wobble
args: freq440,amp1,gate1,sus1,pan0,attT0,decT0,susL1,relT0.2,out0,lfo0.0

amp = amp * 3;
osc = Pulse.ar(freq, mul: amp / 4, width: 0.5) * LFPar.ar(freq - 2.1864, mul:amp, iphase: 0.2) * SinOsc.ar(freq * 6.0012, mul: amp);
osc = osc + SinOsc.ar(freq * 4.012, mul:0.8*amp);
osc = osc * EnvGen.ar(envelope: Env.adsr(attT, decT, susL, relT), gate: gate, doneAction: Done.freeSelf);
osc = LPF.ar(in: osc, freq: 500, mul: 1.0, add: 0.0);
osc = HPF.ar(in: osc, freq: 30, mul: 1.0, add: 0.0);
osc = Pan2.ar(Mix(osc) * 0.5, pan);
Out.ar(out, osc)";

        let result = compile_template(input).unwrap();
        eprintln!("{}", result);

        assert!(result.contains("SynthDef.new(\"wobble\""));
        assert!(result.contains("|freq=440,amp=1,gate=1,sus=1,pan=0,attT=0,decT=0,susL=1,relT=0.2,out=0,lfo=0.0|"));
        assert!(result.contains("var osc;"));
        assert!(!result.contains("var amp,"));
        assert!(result.contains("Out.ar(out, osc)"));
    }

    #[test]
    fn test_load_synthdefs_from_scd() {
        // Create a temp SCD file
        let dir = std::env::temp_dir();
        let path = dir.join("test_synthdefs.scd");
        std::fs::write(
            &path,
            r#"SynthDef.new("blip", {|amp=1,freq=440|
    Out.ar(out, SinOsc.ar(freq, mul: amp))
})

SynthDef.new("pluck", {|amp=1|
    Out.ar(out, Saw.ar(440, mul: amp))
})"#,
        )
        .unwrap();

        let defs = load_synthdefs_from_scd(path.to_str().unwrap()).unwrap();
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "blip");
        assert_eq!(defs[1].name, "pluck");
        assert!(defs[0].content.contains("SynthDef.new(\"blip\""));
        assert!(defs[1].content.contains("SynthDef.new(\"pluck\""));
    }
}
