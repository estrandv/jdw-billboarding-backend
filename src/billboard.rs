use crate::shuttle;
use std::collections::HashMap;

/// A single track in a mini-billboard.
#[derive(Debug, Clone)]
pub struct Track {
    pub name: String,
    pub synth: String,
    pub enabled: bool,
    pub default_args: HashMap<String, f64>,
    pub elements: Vec<shuttle::ResolvedElement>,
}

/// A complete mini-billboard composition.
#[derive(Debug, Clone)]
pub struct Billboard {
    pub tracks: Vec<Track>,
}

/// Parse a mini-billboard text file content.
///
/// Format:
/// ```text
/// # Comment line (track disabled)
/// trackname:synthname arg1=val1,arg2=val2  (c4 d4 e4)*4:amp0.5
/// drums:SP_Roland808  14 14 26 32
/// ```
pub fn parse(source: &str) -> Result<Billboard, String> {
    let mut tracks = Vec::new();

    for (line_no, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        // Comments
        if line.starts_with('#') {
            continue;
        }

        // Parse: trackname:synthname [args] <shuttle_notation>
        let colon_pos = match line.find(':') {
            Some(p) => p,
            None => return Err(format!("Line {}: missing ':' separator", line_no + 1)),
        };

        let name = line[..colon_pos].trim().to_string();
        let rest = &line[colon_pos + 1..].trim();

        // Split synth from shuttle by finding first space that separates
        let (synth_and_args, shuttle_str) = split_synth_and_shuttle(rest)?;

        let (synth, default_args) = parse_synth_with_args(&synth_and_args);

        let elements = if shuttle_str.is_empty() {
            Vec::new()
        } else {
            shuttle::parse(&shuttle_str)?
        };

        tracks.push(Track {
            name,
            synth,
            enabled: true,
            default_args,
            elements,
        });
    }

    Ok(Billboard { tracks })
}

/// Split the RHS of `trackname:rest` into synth+args portion and shuttle notation portion.
fn split_synth_and_shuttle(rest: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Empty content after track name".to_string());
    }

    let synth = parts[0].to_string();
    let mut args_parts: Vec<&str> = Vec::new();
    let mut shuttle_parts: Vec<&str> = Vec::new();
    let mut in_shuttle = false;

    for part in &parts[1..] {
        if in_shuttle {
            shuttle_parts.push(*part);
        } else if part.contains('=') {
            args_parts.push(*part);
        } else {
            in_shuttle = true;
            shuttle_parts.push(*part);
        }
    }

    let mut combined = synth;
    if !args_parts.is_empty() {
        combined.push(' ');
        combined.push_str(&args_parts.join(" "));
    }
    let shuttle = shuttle_parts.join(" ");

    Ok((combined, shuttle))
}

fn parse_synth_with_args(input: &str) -> (String, HashMap<String, f64>) {
    let parts: Vec<&str> = input.splitn(2, char::is_whitespace).collect();
    let synth = parts[0].to_string();
    let mut args = HashMap::new();

    if parts.len() > 1 {
        let args_str = parts[1];
        for arg_pair in args_str.split(',') {
            let pair = arg_pair.trim();
            if let Some(eq_pos) = pair.find('=') {
                let key = pair[..eq_pos].trim().to_string();
                let val_str = pair[eq_pos + 1..].trim();
                if let Ok(val) = val_str.parse::<f64>() {
                    args.insert(key, val);
                }
            }
        }
    }

    (synth, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_billboard() {
        let source = "moogBass:default c4 d4 e4 f4\ndrums:SP_Roland808 14 14 26 32\n#comment:disabled\nkeys:FMRhodes (c5 e5 g5)*2";
        let bb = parse(source).unwrap();
        assert_eq!(bb.tracks.len(), 3);
        assert!(bb.tracks[2].enabled);
        assert_eq!(bb.tracks[0].name, "moogBass");
        assert_eq!(bb.tracks[0].synth, "default");
        assert_eq!(bb.tracks[0].elements.len(), 4);
        assert_eq!(bb.tracks[1].elements.len(), 4);
        assert_eq!(bb.tracks[2].elements.len(), 6); // (c5 e5 g5)*2 = 6
    }

    #[test]
    fn test_with_args() {
        let source = "bass:eBass amp=0.5,sus=1.0 (c3 g3)*2";
        let bb = parse(source).unwrap();
        assert_eq!(bb.tracks[0].default_args.get("amp"), Some(&0.5));
        assert_eq!(bb.tracks[0].synth, "eBass");
        assert_eq!(bb.tracks[0].elements.len(), 4);
    }
}
