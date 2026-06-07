/// Macro template language for .bbd code generation.
///
/// Syntax:
///     $plain = 1 2 3
///     $with_args(arg1, arg2) = text $:arg1 and $:arg2

use regex::Regex;

#[derive(Debug, Clone)]
pub struct MacroDefinition {
    pub name: String,
    pub args: Vec<String>,
    pub template: String,
}

#[derive(Debug, Clone)]
pub struct MacroCall {
    pub name: String,
    pub args: Vec<String>,
    pub source: String,
}

/// Find all macro definition lines.
pub fn find_macro_defs(source: &str) -> Vec<String> {
    let re = Regex::new(
        r"\$[a-z|_]+(?:\([0-9a-zA-Z,]+\))?\s+?=\s+?(.*)",
    )
    .unwrap();
    re.find_iter(source)
        .map(|m| m.as_str().to_string())
        .collect()
}

/// Find all macro call references (definitions/calls with $name or $name(...)).
pub fn find_macro_calls(source: &str) -> Vec<String> {
    let re = Regex::new(r"\$[a-z|_]+(?:\([0-9a-zA-Z,]+\))?").unwrap();
    re.find_iter(source)
        .map(|m| m.as_str().to_string())
        .collect()
}

/// Parse a macro definition line.
pub fn parse_macro_def(source_line: &str) -> MacroDefinition {
    let eq_pos = source_line.find('=').expect("macro def must contain =");
    let func_def = source_line[..eq_pos].trim();
    let content_def = source_line[eq_pos + 1..].trim();

    let args = if let Some(paren_start) = func_def.find('(') {
        let paren_end = func_def.rfind(')').expect("unmatched paren in macro def");
        func_def[paren_start + 1..paren_end]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        Vec::new()
    };

    let name_re = Regex::new(r"\$([a-zA-Z|_]+)").unwrap();
    let name = name_re
        .captures(func_def)
        .expect("macro def must have a name")
        .get(1)
        .unwrap()
        .as_str()
        .to_string();

    MacroDefinition {
        name,
        args,
        template: content_def.to_string(),
    }
}

/// Parse a macro call text.
pub fn parse_macro_call(text: &str) -> MacroCall {
    let args = if text.contains('(') {
        let paren_start = text.find('(').expect("macro call has (");
        let paren_end = text.rfind(')').expect("unmatched paren in macro call");
        text[paren_start + 1..paren_end]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        Vec::new()
    };

    let name_re = Regex::new(r"\$([a-zA-Z|_]+)").unwrap();
    let name = name_re
        .captures(text)
        .expect("macro call must have a name")
        .get(1)
        .unwrap()
        .as_str()
        .to_string();

    MacroCall {
        name,
        args,
        source: text.to_string(),
    }
}

/// Resolve a macro call against definitions.
pub fn resolve_macro(
    call: &MacroCall,
    definitions: &[MacroDefinition],
) -> Result<String, String> {
    let filtered: Vec<&MacroDefinition> = definitions.iter().filter(|d| d.name == call.name).collect();
    let def = filtered
        .first()
        .ok_or_else(|| format!("undefined macro: ${}", call.name))?;

    let mut template = def.template.clone();
    for (i, arg_value) in call.args.iter().enumerate() {
        let placeholder = format!("$:{}", def.args[i]);
        template = template.replace(&placeholder, arg_value);
    }

    Ok(template)
}

/// Compile macros: remove def lines, resolve all calls.
pub fn compile_macros(text: &str, supplied_defs: &[String]) -> Result<String, String> {
    let mut defs_text = Vec::new();

    // Sort definition lines by length descending to avoid substring conflicts
    let mut def_lines = find_macro_defs(text);
    def_lines.sort_by(|a, b| b.len().cmp(&a.len()));

    defs_text.extend(def_lines);
    defs_text.extend(supplied_defs.iter().cloned());

    // Remove definition lines from text
    let mut defs_removed = text.to_string();
    for d in &defs_text {
        defs_removed = defs_removed.replace(d.as_str(), "");
    }

    // Parse definitions
    let definitions: Vec<MacroDefinition> = defs_text.iter().map(|s| parse_macro_def(s)).collect();

    // Find and resolve calls
    let calls: Vec<MacroCall> = find_macro_calls(&defs_removed)
        .iter()
        .map(|c| parse_macro_call(c))
        .collect();

    let mut end_text = defs_removed;
    for c in &calls {
        let resolved = resolve_macro(c, &definitions)?;
        end_text = end_text.replace(&c.source, &resolved);
    }

    Ok(end_text)
}

/// Load a .bbd file and its sibling common_macros.txt, expand macros.
pub fn load_and_expand(bbd_path: &str) -> Result<String, String> {
    let content =
        std::fs::read_to_string(bbd_path).map_err(|e| format!("read {}: {}", bbd_path, e))?;

    // Try loading sibling common_macros.txt
    let macro_dir = std::path::Path::new(bbd_path).parent().unwrap_or(std::path::Path::new("."));
    let macros_path = macro_dir.join("common_macros.txt");
    let mut supplied_defs = Vec::new();

    if macros_path.exists() {
        let common_content =
            std::fs::read_to_string(&macros_path).map_err(|e| format!("read {:?}: {}", macros_path, e))?;
        supplied_defs = find_macro_defs(&common_content);
    }

    compile_macros(&content, &supplied_defs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_macro() {
        let source = "$fish = Sometimes I dream of fish
$fish";
        let result = compile_macros(source, &[]).unwrap();
        assert_eq!(result.trim(), "Sometimes I dream of fish");
    }

    #[test]
    fn test_macro_with_args() {
        let source = "$more_than(times) = more than $:times times
$more_than(6)";
        let result = compile_macros(source, &[]).unwrap();
        assert_eq!(result.trim(), "more than 6 times");
    }

    #[test]
    fn test_macro_with_multiple_args() {
        let source =         "$greet(name,count) = Hello $:name you have $:count messages
$greet(Alice,3)";
        let result = compile_macros(source, &[]).unwrap();
        assert_eq!(result.trim(), "Hello Alice you have 3 messages");
    }

    #[test]
    fn test_supplied_defs() {
        let source = "$chug(99)";
        let defs = vec!["$chug(note) = $:note*3:0.5".to_string()];
        let result = compile_macros(source, &defs).unwrap();
        assert_eq!(result.trim(), "99*3:0.5");
    }

    #[test]
    fn test_chord_macro() {
        let source = "$cmajarp(octave) = c$:octave e$:octave g$:octave
$cmajarp(5)";
        let result = compile_macros(source, &[]).unwrap();
        assert_eq!(result.trim(), "c5 e5 g5");
    }

    #[test]
    fn test_multiple_calls() {
        let source = "$cmajarp(octave) = c$:octave e$:octave g$:octave
$cmajarp(4) $cmajarp(5)";
        let result = compile_macros(source, &[]).unwrap();
        assert_eq!(result.trim(), "c4 e4 g4 c5 e5 g5");
    }
}
