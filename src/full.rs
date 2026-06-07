/// Full Billboard Notation parser.
///
/// Stages (per PLAN_full_billboard_parser.md):
///   Stage 1 — Line classifier + continuation + inline comments
///   Stage 2 — Section grouper
///   Stage 3 — Low-level parsers
///   Stage 4 — Billboard construction + argument inheritance       ← current
///   Stage 5 — OSC conversion
///   Stage 6 — jdw-suite integration
use std::collections::HashMap;
use std::fmt;

/// Line types after syntactic classification.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineType {
    GroupFilter,
    SynthHeader,
    TrackDefinition,
    EffectDefinition,
    Command,
    DefaultStatement,
    Comment,
}

impl fmt::Display for LineType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LineType::GroupFilter => write!(f, "group_filter"),
            LineType::SynthHeader => write!(f, "synth_header"),
            LineType::TrackDefinition => write!(f, "track"),
            LineType::EffectDefinition => write!(f, "effect"),
            LineType::Command => write!(f, "command"),
            LineType::DefaultStatement => write!(f, "default"),
            LineType::Comment => write!(f, "comment"),
        }
    }
}

/// A single classified line.
#[derive(Debug, Clone)]
pub struct ClassifiedLine {
    pub line_type: LineType,
    /// Content after stripping inline comments and leading/trailing whitespace.
    pub content: String,
    /// Raw line number (1-indexed, after continuation joining).
    pub line_number: usize,
    /// The inline comment text (without leading `#`), if any.
    pub inline_comment: Option<String>,
}

/// Join lines connected by trailing backslash continuation.
///
/// A line ending with `\` (possibly with trailing whitespace before the newline)
/// is joined with the next line. The backslash and newline are removed.
pub fn join_continuations<'a>(raw_lines: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut joined = Vec::new();
    let mut current = String::new();

    for line in raw_lines {
        if current.is_empty() {
            current = line.to_string();
        } else {
            current.push_str(line);
        }

        // Check if the line ends with a continuation backslash.
        // Remove the backslash and any trailing whitespace between it and
        // the newline, but preserve all other whitespace.
        if let Some(bs_idx) = current.rfind('\\') {
            // The backslash must be at the end (after stripping trailing whitespace
            // that separates it from the newline)
            let after_bs = &current[bs_idx + 1..];
            if after_bs.trim().is_empty() {
                // Remove the backslash and whitespace between it and newline
                current = current[..bs_idx].to_string();
                continue;
            }
        }

        joined.push(current.clone());
        current.clear();
    }

    // If there's an unclosed continuation, treat the unterminated `\` as literal
    if !current.is_empty() {
        joined.push(current);
    }

    joined
}

/// Split a line into its content and optional trailing inline comment.
///
/// An inline comment starts with `#` that is either at the start of the content
/// (making it a full-line comment) or preceded by whitespace.
/// Returns `(content, Some(comment_text))` or `(content, None)`.
pub fn split_inline_comment(line: &str) -> (&str, Option<&str>) {
    for (i, b) in line.as_bytes().iter().enumerate() {
        if *b == b'#' && (i == 0 || line.as_bytes()[i - 1] == b' ' || line.as_bytes()[i - 1] == b'\t')
        {
            return (&line[..i], Some(&line[i + 1..]));
        }
    }
    (line, None)
}

/// Classify a single line (content only, after comment stripping).
///
/// Returns `None` for empty/whitespace-only content.
pub fn classify_content(content: &str) -> Option<LineType> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.starts_with(">>>") {
        Some(LineType::GroupFilter)
    } else if trimmed.starts_with("*@") || trimmed.starts_with('@') {
        Some(LineType::SynthHeader)
    } else if trimmed.starts_with('€') {
        Some(LineType::EffectDefinition)
    } else if trimmed.starts_with("DEFAULT") {
        let rest = &trimmed[7..];
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            Some(LineType::DefaultStatement)
        } else {
            Some(LineType::TrackDefinition)
        }
    } else if trimmed.starts_with('/') {
        Some(LineType::Command)
    } else if trimmed.starts_with("COMMAND")
        || trimmed.starts_with("UPDATE_COMMAND")
        || trimmed.starts_with("QUEUE_COMMAND")
    {
        Some(LineType::Command)
    } else if trimmed.starts_with('#') {
        Some(LineType::Comment)
    } else {
        Some(LineType::TrackDefinition)
    }
}

/// Classify raw billboard source lines into `ClassifiedLine` entries.
///
/// Handles continuation joining and inline comment stripping.
pub fn classify_source(source: &str) -> Vec<ClassifiedLine> {
    let raw_lines: Vec<&str> = source.lines().collect();
    let joined = join_continuations(raw_lines);
    let mut result = Vec::new();

    for (i, line) in joined.iter().enumerate() {
        let trimmed = line.trim();
        let line_number = i + 1;

        if trimmed.is_empty() {
            continue;
        }

        // Split off inline comment
        let (content_raw, inline_comment) = split_inline_comment(trimmed);
        let content = content_raw.trim();

        // If the content is a full-line comment (starts with #)
        if content.starts_with('#') {
            result.push(ClassifiedLine {
                line_type: LineType::Comment,
                content: content.to_string(),
                line_number,
                inline_comment: None,
            });
            continue;
        }

        if content.is_empty() && inline_comment.is_some() {
            // Line was only a comment
            result.push(ClassifiedLine {
                line_type: LineType::Comment,
                content: format!("#{}", inline_comment.unwrap()),
                line_number,
                inline_comment: None,
            });
            continue;
        }

        if content.is_empty() {
            continue;
        }

        let line_type = classify_content(content).unwrap_or(LineType::Comment);
        result.push(ClassifiedLine {
            line_type,
            content: content.to_string(),
            line_number,
            inline_comment: inline_comment.map(|s| s.trim().to_string()),
        });
    }

    result
}

// ---------------------------------------------------------------------------
// Stage 2 — Section Grouper
// ---------------------------------------------------------------------------

/// A group of lines belonging to one synth section.
#[derive(Debug, Clone)]
pub struct SectionGroup {
    pub header: ClassifiedLine,
    pub tracks: Vec<ClassifiedLine>,
    pub effects: Vec<ClassifiedLine>,
    pub comments: Vec<ClassifiedLine>,
}

/// Top-level parsed structure: filters, default, commands, sections.
#[derive(Debug, Clone)]
pub struct GroupedBillboard {
    /// First unbroken chain of group filters.
    pub filters: Vec<ClassifiedLine>,
    /// The last `DEFAULT` statement, if any.
    pub default_statement: Option<ClassifiedLine>,
    /// All command lines.
    pub commands: Vec<ClassifiedLine>,
    /// Synth sections, in order.
    pub sections: Vec<SectionGroup>,
    /// Lines that appeared before any section header (outside filter/default/range).
    pub orphan_lines: Vec<ClassifiedLine>,
}

/// Walk classified lines and group them into sections + top-level items.
///
/// Rules:
/// - Group filters are collected only from the first unbroken chain
///   (comments don't break the chain; any non-filter, non-comment line does).
/// - The last `DEFAULT` statement is recorded.
/// - All `COMMAND`/`UPDATE_COMMAND`/`QUEUE_COMMAND` lines are collected.
/// - Track and Effect lines are grouped under their most recent `SynthHeader`.
/// - Orphan tracks/effects (before any header) are stored separately.
pub fn group_sections(classified: &[ClassifiedLine]) -> GroupedBillboard {
    let mut filters = Vec::new();
    let mut default_statement = None;
    let mut commands = Vec::new();
    let mut sections: Vec<SectionGroup> = Vec::new();
    let mut orphan_lines = Vec::new();

    let mut in_filter_chain = true;
    let mut current_section: Option<SectionGroup> = None;

    for line in classified {
        match line.line_type {
            LineType::GroupFilter => {
                if in_filter_chain {
                    filters.push(line.clone());
                } else {
                    // Filter after chain is broken → orphan
                    orphan_lines.push(line.clone());
                }
            }

            LineType::DefaultStatement => {
                in_filter_chain = false;
                default_statement = Some(line.clone());
            }

            LineType::Command => {
                in_filter_chain = false;
                commands.push(line.clone());
            }

            LineType::SynthHeader => {
                in_filter_chain = false;
                // Close previous section
                if let Some(prev) = current_section.take() {
                    sections.push(prev);
                }
                current_section = Some(SectionGroup {
                    header: line.clone(),
                    tracks: Vec::new(),
                    effects: Vec::new(),
                    comments: Vec::new(),
                });
            }

            LineType::TrackDefinition => {
                in_filter_chain = false;
                match &mut current_section {
                    Some(s) => s.tracks.push(line.clone()),
                    None => orphan_lines.push(line.clone()),
                }
            }

            LineType::EffectDefinition => {
                in_filter_chain = false;
                match &mut current_section {
                    Some(s) => s.effects.push(line.clone()),
                    None => orphan_lines.push(line.clone()),
                }
            }

            LineType::Comment => {
                // Comments don't break the filter chain.
                // If we're inside a section, store them for index tracking.
                if let Some(s) = &mut current_section {
                    s.comments.push(line.clone());
                }
                // Otherwise just ignore (don't break filter chain).
            }
        }
    }

    // Push the last section
    if let Some(last) = current_section {
        sections.push(last);
    }

    GroupedBillboard {
        filters,
        default_statement,
        commands,
        sections,
        orphan_lines,
    }
}

// ---------------------------------------------------------------------------
// Stage 3 — Low-Level Parsers
// ---------------------------------------------------------------------------

/// Parsed synth header data from `[@|*@][SP_|DR_]instrument[:group] [args] [pad_config]`.
#[derive(Debug, Clone, PartialEq)]
pub struct SynthHeaderData {
    pub instrument: String,
    pub is_drone: bool,
    pub is_sampler: bool,
    pub is_selected: bool,
    pub group: Option<String>,
    pub args: Vec<(String, String)>,
    /// Sampler pad config: `[(index, sample_id), ...]`
    pub pad_config: Vec<(u32, u32)>,
}

/// Parsed track metadata from `<group[;arg1,arg2]>`.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackMeta {
    pub group_override: Option<String>,
    /// `(key, operator, value)` — operator is one of `=`, `+`, `-`, `*`.
    pub arg_overrides: Vec<(String, String, String)>,
}

/// Parsed effect definition from `€type:id [args]`.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectData {
    pub effect_type: String,
    pub id: String,
    pub args: Vec<(String, String)>,
}

/// Command context.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CommandContext {
    All,
    Update,
    Queue,
}

/// Parsed command from `[COMMAND_TYPE] /address [args...]`.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandData {
    pub context: CommandContext,
    pub address: String,
    pub args: Vec<String>,
}

/// Parsed group filter from `>>> name1 name2 ...`.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterData {
    pub groups: Vec<String>,
}

// -- Arg list parser (shared) --

/// Parse a comma-separated arg string like `amp0.5,sus1.0,time0.5`.
///
/// Each token is `key`, `key=value`, `key+value`, or an implicit
/// `<name><number>` pair like `amp0.5`.
/// Returns `Vec<(key, operator, value)>` where operator is one of
/// `=`, `+`, `-`, `*`, or `_` (implicit/no operator).
pub fn parse_arg_list(s: &str) -> Vec<(String, String, String)> {
    if s.trim().is_empty() {
        return Vec::new();
    }
    s.split(',')
        .map(|token| {
            let token = token.trim();
            if token.is_empty() {
                return ("_".to_string(), String::new(), String::new());
            }
            // Find explicit operator position
            for (i, b) in token.as_bytes().iter().enumerate() {
                if *b == b'=' || *b == b'+' || *b == b'-' || *b == b'*' {
                    let key = token[..i].trim().to_string();
                    let op = token[i..=i].to_string();
                    let val = token[i + 1..].trim().to_string();
                    return (key, op, val);
                }
            }
            // No explicit operator — try splitting as implicit <name><number>.
            // Name is [A-Za-z_]+; number starts at first digit.
            if let Some(digit_start) = token.find(|c: char| c.is_ascii_digit()) {
                let before = &token[..digit_start];
                let after = &token[digit_start..];
                if !before.is_empty() && before.chars().all(|c| c.is_ascii_alphabetic() || c == '_') {
                    return (before.to_string(), "_".to_string(), after.to_string());
                }
            }
            // Bare value (no name)
            (token.to_string(), "_".to_string(), String::new())
        })
        .collect()
}

/// Parse a simple `key=val` arg list (no operators), returning `HashMap`-like pairs.
pub fn parse_simple_args(s: &str) -> Vec<(String, String)> {
    parse_arg_list(s)
        .into_iter()
        .map(|(k, _op, v)| (k, v))
        .collect()
}

// -- Synth header parser --

/// Parse a synth header line (without leading `@`/`*@`).
///
/// Expected format: `[SP_|DR_]instrument[:group] [args] [pad_config]`
fn parse_synth_header_inner(content: &str, is_selected: bool) -> Result<SynthHeaderData, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("Empty synth header".to_string());
    }

    let rest = trimmed;

    // Check for SP_ / DR_ prefix
    let is_sampler = rest.starts_with("SP_");
    let is_drone = rest.starts_with("DR_");
    let after_prefix = if is_sampler || is_drone {
        &rest[3..]
    } else {
        rest
    };

    // Extract instrument name (up to ':', space, or end)
    let mut instrument = String::new();
    let mut group = None;
    let mut cursor = 0;
    let bytes = after_prefix.as_bytes();

    while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() && bytes[cursor] != b':' {
        instrument.push(bytes[cursor] as char);
        cursor += 1;
    }

    if instrument.is_empty() {
        return Err("Empty instrument name in synth header".to_string());
    }

    // Check for :group
    if cursor < bytes.len() && bytes[cursor] == b':' {
        cursor += 1; // skip ':'
        let mut g = String::new();
        while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() {
            g.push(bytes[cursor] as char);
            cursor += 1;
        }
        if !g.is_empty() {
            group = Some(g);
        }
    }

    // Remaining: args and pad_config
    let remainder = after_prefix[cursor..].trim();
    let mut args = Vec::new();
    let mut pad_config = Vec::new();

                if !remainder.is_empty() {
        // Try to split into arg portion vs pad config portion.
        // Pad config looks like `1:0 2:14` — space-separated num:num pairs.
        // Args look like `amp0.5,sus1.0` — comma-separated key=val or
        // single tokens like `amp0.5`.
        let tokens: Vec<&str> = remainder.split_whitespace().collect();
        let mut in_pad_config = false;

        for token in &tokens {
            if in_pad_config {
                if let Some((idx_str, sid_str)) = token.split_once(':') {
                    if let (Ok(idx), Ok(sid)) = (idx_str.parse::<u32>(), sid_str.parse::<u32>()) {
                        pad_config.push((idx, sid));
                        continue;
                    }
                }
                args.extend(parse_simple_args(token));
            } else if let Some((idx_str, sid_str)) = token.split_once(':') {
                if idx_str.chars().all(|c| c.is_ascii_digit())
                    && sid_str.chars().all(|c| c.is_ascii_digit())
                {
                    if let (Ok(idx), Ok(sid)) = (idx_str.parse::<u32>(), sid_str.parse::<u32>()) {
                        pad_config.push((idx, sid));
                        in_pad_config = true;
                        continue;
                    }
                }
                args.extend(parse_simple_args(token));
            } else {
                // Any other token: try parsing as args
                args.extend(parse_simple_args(token));
            }
        }
    }

    Ok(SynthHeaderData {
        instrument,
        is_drone,
        is_sampler,
        is_selected,
        group,
        args,
        pad_config,
    })
}

/// Parse a synth header line: `[@|*@][SP_|DR_]instrument[:group] [args]`.
pub fn parse_synth_header(content: &str) -> Result<SynthHeaderData, String> {
    let trimmed = content.trim();
    let is_selected = trimmed.starts_with("*@");
    if !trimmed.starts_with('@') && !trimmed.starts_with("*@") {
        return Err("Synth header must start with @ or *@".to_string());
    }

    let inner = if is_selected {
        &trimmed[2..]
    } else {
        &trimmed[1..]
    };

    parse_synth_header_inner(inner, is_selected)
}

// -- Track metadata parser --

/// Parse track metadata from `<group[;arg1,arg2]>`.
///
/// If the line doesn't start with `<`, returns `None` (no metadata).
pub fn parse_track_metadata(content: &str) -> Option<TrackMeta> {
    let trimmed = content.trim();
    if !trimmed.starts_with('<') {
        return None;
    }

    // Find the closing '>'
    let close = trimmed.find('>')?;
    let inner = &trimmed[1..close];

    let (group_override, arg_str) = match inner.split_once(';') {
        Some((g, a)) => (Some(g.trim().to_string()), a),
        None => (Some(inner.trim().to_string()), ""),
    };

    let group_override = if group_override.as_ref().map_or(true, |s| s.is_empty()) {
        None
    } else {
        group_override
    };

    let arg_overrides = if arg_str.is_empty() {
        Vec::new()
    } else {
        parse_arg_list(arg_str)
            .into_iter()
            .map(|(k, op, v)| (k, op, v))
            .collect()
    };

    Some(TrackMeta {
        group_override,
        arg_overrides,
    })
}

// -- Effect definition parser --

/// Parse an effect definition: `€type:id [args]`.
pub fn parse_effect(content: &str) -> Result<EffectData, String> {
    let trimmed = content.trim();
    if !trimmed.starts_with('€') {
        return Err("Effect must start with €".to_string());
    }

    // Skip the € character (multi-byte in UTF-8, use char_indices)
    let after_euro = &trimmed[trimmed.char_indices().nth(1).map(|(i, _)| i).unwrap_or(trimmed.len())..];

    // Find ':' separating type from id
    let colon = after_euro
        .find(':')
        .ok_or_else(|| "Effect definition missing ':'".to_string())?;

    let effect_type = after_euro[..colon].trim().to_string();
    let rest = after_euro[colon + 1..].trim();

    // Split id from args on whitespace
    let (id, args_str) = match rest.find(char::is_whitespace) {
        Some(pos) => (rest[..pos].trim().to_string(), rest[pos..].trim()),
        None => (rest.to_string(), ""),
    };

    let args = parse_simple_args(args_str);

    Ok(EffectData {
        effect_type,
        id,
        args,
    })
}

// -- Command parser --

/// Parse a command: `[COMMAND_TYPE] /address [args...]`.
pub fn parse_command(content: &str) -> Result<CommandData, String> {
    let trimmed = content.trim();

    let (context, rest) = if trimmed.starts_with('/') {
        // Bare /address — no COMMAND type prefix
        (CommandContext::All, trimmed)
    } else if trimmed.starts_with("UPDATE_COMMAND") {
        (CommandContext::Update, trimmed[14..].trim())
    } else if trimmed.starts_with("QUEUE_COMMAND") {
        (CommandContext::Queue, trimmed[13..].trim())
    } else if trimmed.starts_with("COMMAND") {
        let after = &trimmed[7..];
        if after.is_empty() || after.starts_with(char::is_whitespace) {
            (CommandContext::All, after.trim())
        } else {
            return Err(format!("Unknown command prefix in: {}", trimmed));
        }
    } else {
        return Err(format!("Command does not start with COMMAND: {}", trimmed));
    };

    let tokens: Vec<&str> = rest.split_whitespace().collect();
    if tokens.is_empty() {
        return Err("Command missing address".to_string());
    }

    let address = tokens[0].to_string();
    if !address.starts_with('/') {
        return Err(format!("Command address must start with '/': {}", address));
    }

    let args: Vec<String> = tokens[1..].iter().map(|s| s.to_string()).collect();

    Ok(CommandData {
        context,
        address,
        args,
    })
}

// -- Group filter parser --

/// Parse a group filter: `>>> name1 name2 ...`.
pub fn parse_filter(content: &str) -> Result<FilterData, String> {
    let trimmed = content.trim();
    if !trimmed.starts_with(">>>") {
        return Err("Filter must start with >>>".to_string());
    }

    let rest = trimmed[3..].trim();
    if rest.is_empty() {
        return Ok(FilterData {
            groups: Vec::new(),
        });
    }

    let groups: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
    Ok(FilterData { groups })
}

// -- Convenience: parse all low-level structures from a GroupedBillboard --

/// Parse all synth headers in a `GroupedBillboard`, returning errors per section.
pub fn parse_all_headers(g: &GroupedBillboard) -> Vec<Result<SynthHeaderData, String>> {
    g.sections
        .iter()
        .map(|sec| parse_synth_header(&sec.header.content))
        .collect()
}

/// Parse all track metadata lines in a section.
pub fn parse_tracks_metadata(section: &SectionGroup) -> Vec<Option<TrackMeta>> {
    section
        .tracks
        .iter()
        .map(|t| parse_track_metadata(&t.content))
        .collect()
}

/// Parse all effects in a section.
pub fn parse_section_effects(section: &SectionGroup) -> Vec<Result<EffectData, String>> {
    section
        .effects
        .iter()
        .map(|e| parse_effect(&e.content))
        .collect()
}

/// Parse all commands.
pub fn parse_all_commands(g: &GroupedBillboard) -> Vec<Result<CommandData, String>> {
    g.commands
        .iter()
        .map(|c| parse_command(&c.content))
        .collect()
}

/// Parse all filters.
pub fn parse_all_filters(g: &GroupedBillboard) -> Vec<Result<FilterData, String>> {
    g.filters
        .iter()
        .map(|f| parse_filter(&f.content))
        .collect()
}

// ---------------------------------------------------------------------------
// Stage 4 — Billboard Construction + Argument Inheritance
// ---------------------------------------------------------------------------

/// Final, resolved synth header.
#[derive(Debug, Clone)]
pub struct SynthHeader {
    pub instrument: String,
    pub is_drone: bool,
    pub is_sampler: bool,
    pub is_selected: bool,
    pub group: Option<String>,
    pub default_args: HashMap<String, f64>,
    pub pad_config: Vec<(u32, u32)>,
}

/// Final, resolved track definition.
#[derive(Debug, Clone)]
pub struct TrackDefinition {
    /// Raw shuttle notation content (after stripping `<meta>` prefix).
    pub content: String,
    pub group_override: Option<String>,
    /// Arg overrides with operator: `(op_char, value)`.
    pub arg_overrides: HashMap<String, (char, f64)>,
    pub index: usize,
    pub enabled: bool,
}

/// Final, resolved effect definition.
#[derive(Debug, Clone)]
pub struct EffectDefinition {
    pub effect_type: String,
    pub id: String,
    pub args: HashMap<String, f64>,
}

/// Final, resolved synth section.
#[derive(Debug, Clone)]
pub struct SynthSection {
    pub header: SynthHeader,
    pub tracks: Vec<TrackDefinition>,
    pub effects: Vec<EffectDefinition>,
}

/// Final billboard command.
#[derive(Debug, Clone)]
pub struct BillboardCommand {
    pub context: CommandContext,
    pub address: String,
    pub args: Vec<String>,
}

/// The final parsed Billboard.
#[derive(Debug, Clone)]
pub struct Billboard {
    pub sections: Vec<SynthSection>,
    pub filters: Vec<Vec<String>>,
    pub commands: Vec<BillboardCommand>,
    pub default_args: HashMap<String, f64>,
}

/// Resolve argument inheritance chain.
///
/// Priority (lowest → highest):
/// 1. `defaults` — global `DEFAULT` statement
/// 2. `header_args` — synth header line args
/// 3. `track_overrides` — track metadata operators
///
/// Operators:
/// - `_` or `=`: Replace
/// - `+`: Add to inherited
/// - `-`: Subtract from inherited
/// - `*`: Multiply inherited
pub fn resolve_args(
    defaults: &HashMap<String, f64>,
    header_args: &HashMap<String, f64>,
    track_overrides: &HashMap<String, (char, f64)>,
) -> HashMap<String, f64> {
    let mut result = defaults.clone();
    for (k, v) in header_args {
        result.insert(k.clone(), *v);
    }
    for (k, &(op, v)) in track_overrides {
        match op {
            '=' | '_' => {
                result.insert(k.clone(), v);
            }
            '+' => {
                *result.entry(k.clone()).or_insert(0.0) += v;
            }
            '-' => {
                *result.entry(k.clone()).or_insert(0.0) -= v;
            }
            '*' => {
                *result.entry(k.clone()).or_insert(1.0) *= v;
            }
            _ => {
                result.insert(k.clone(), v);
            }
        }
    }
    result
}

/// Build a `Billboard` from a `GroupedBillboard`.
pub fn build_billboard(grouped: &GroupedBillboard) -> Billboard {
    // Parse defaults
    let default_args: HashMap<String, f64> = grouped
        .default_statement
        .as_ref()
        .map(|d| {
            let rest = d.content[7..].trim();
            parse_simple_args(rest)
                .into_iter()
                .filter_map(|(k, v)| v.parse::<f64>().ok().map(|fv| (k, fv)))
                .collect()
        })
        .unwrap_or_default();

    // Parse filters
    let filters: Vec<Vec<String>> = grouped
        .filters
        .iter()
        .filter_map(|f| parse_filter(&f.content).ok())
        .map(|fd| fd.groups)
        .collect();

    // Parse commands
    let commands: Vec<BillboardCommand> = grouped
        .commands
        .iter()
        .filter_map(|c| parse_command(&c.content).ok())
        .map(|cd| BillboardCommand {
            context: cd.context,
            address: cd.address,
            args: cd.args,
        })
        .collect();

    // Build sections
    let sections: Vec<SynthSection> = grouped
        .sections
        .iter()
        .filter_map(|sec| {
            let header_data = parse_synth_header(&sec.header.content).ok()?;
            let header = SynthHeader {
                instrument: header_data.instrument,
                is_drone: header_data.is_drone,
                is_sampler: header_data.is_sampler,
                is_selected: header_data.is_selected,
                group: header_data.group,
                default_args: header_data
                    .args
                    .into_iter()
                    .filter_map(|(k, v)| v.parse::<f64>().ok().map(|fv| (k, fv)))
                    .collect(),
                pad_config: header_data.pad_config,
            };

            let tracks: Vec<TrackDefinition> = sec
                .tracks
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let meta = parse_track_metadata(&t.content);

                    let content = if meta.is_some() {
                        let close = t.content.find('>').unwrap_or(0);
                        t.content[close + 1..].trim().to_string()
                    } else {
                        t.content.clone()
                    };

                    TrackDefinition {
                        content,
                        group_override: meta.as_ref().and_then(|m| m.group_override.clone()),
                        arg_overrides: meta
                            .map(|m| {
                                m.arg_overrides
                                    .into_iter()
                                    .filter_map(|(k, op, v)| {
                                        v.parse::<f64>().ok().map(|fv| {
                                            (k, (op.chars().next().unwrap_or('='), fv))
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        index: i,
                        enabled: true,
                    }
                })
                .collect();

            let effects: Vec<EffectDefinition> = sec
                .effects
                .iter()
                .filter_map(|e| {
                    parse_effect(&e.content).ok().map(|ed| {
                        let args: HashMap<String, f64> = ed
                            .args
                            .into_iter()
                            .filter_map(|(k, v)| v.parse::<f64>().ok().map(|fv| (k, fv)))
                            .collect();
                        EffectDefinition {
                            effect_type: ed.effect_type,
                            id: ed.id,
                            args,
                        }
                    })
                })
                .collect();

            Some(SynthSection {
                header,
                tracks,
                effects,
            })
        })
        .collect();

    Billboard {
        sections,
        filters,
        commands,
        default_args,
    }
}

/// Full pipeline: classify → group → build Billboard.
pub fn parse(source: &str) -> Billboard {
    let classified = classify_source(source);
    let grouped = group_sections(&classified);
    build_billboard(&grouped)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- join_continuations --

    #[test]
    fn test_no_continuation() {
        let lines = vec!["hello", "world"];
        assert_eq!(join_continuations(lines), vec!["hello", "world"]);
    }

    #[test]
    fn test_basic_continuation() {
        let lines = vec!["hello \\", "world"];
        assert_eq!(join_continuations(lines), vec!["hello world"]);
    }

    #[test]
    fn test_continuation_with_trailing_whitespace() {
        let lines = vec!["hello \\  ", "world"];
        assert_eq!(join_continuations(lines), vec!["hello world"]);
    }

    #[test]
    fn test_multi_line_continuation() {
        let lines = vec!["a \\", "b \\", "c"];
        assert_eq!(join_continuations(lines), vec!["a b c"]);
    }

    #[test]
    fn test_mixed_continuation() {
        let lines = vec!["a \\", "b", "c \\", "d"];
        assert_eq!(join_continuations(lines), vec!["a b", "c d"]);
    }

    #[test]
    fn test_continuation_with_multiple_segments() {
        let lines = vec!["(c4 d4 e4 f4 \\", " g4 a4 b4 c5):amp0.5"];
        assert_eq!(
            join_continuations(lines),
            vec!["(c4 d4 e4 f4  g4 a4 b4 c5):amp0.5"]
        );
    }

    // -- split_inline_comment --

    #[test]
    fn test_no_comment() {
        assert_eq!(split_inline_comment("@synth"), ("@synth", None));
    }

    #[test]
    fn test_inline_comment() {
        assert_eq!(
            split_inline_comment("@synth # this is a comment"),
            ("@synth ", Some(" this is a comment"))
        );
    }

    #[test]
    fn test_full_line_comment() {
        assert_eq!(
            split_inline_comment("# this is a comment"),
            ("", Some(" this is a comment"))
        );
    }

    #[test]
    fn test_sharp_in_note_name_not_comment() {
        // f#5 should not be split as comment since # is not preceded by space
        assert_eq!(split_inline_comment("f#5"), ("f#5", None));
    }

    #[test]
    fn test_comment_after_shuttle() {
        assert_eq!(
            split_inline_comment("c4 d4 # play the notes"),
            ("c4 d4 ", Some(" play the notes"))
        );
    }

    // -- classify_content --

    #[test]
    fn test_empty_content() {
        assert_eq!(classify_content(""), None);
        assert_eq!(classify_content("  "), None);
    }

    #[test]
    fn test_group_filter() {
        assert_eq!(classify_content(">>> drums bass"), Some(LineType::GroupFilter));
        assert_eq!(classify_content(">>>"), Some(LineType::GroupFilter));
    }

    #[test]
    fn test_synth_header() {
        assert_eq!(classify_content("@moogBass"), Some(LineType::SynthHeader));
        assert_eq!(classify_content("@moogBass:bass amp0.5"), Some(LineType::SynthHeader));
        assert_eq!(classify_content("*@SP_Roland808:drums"), Some(LineType::SynthHeader));
    }

    #[test]
    fn test_effect_definition() {
        assert_eq!(classify_content("€reverb:main room0.9"), Some(LineType::EffectDefinition));
    }

    #[test]
    fn test_default_statement() {
        assert_eq!(classify_content("DEFAULT amp0.5"), Some(LineType::DefaultStatement));
        assert_eq!(classify_content("DEFAULT"), Some(LineType::DefaultStatement));
    }

    #[test]
    fn test_command() {
        assert_eq!(classify_content("COMMAND /set_bpm 120"), Some(LineType::Command));
        assert_eq!(classify_content("UPDATE_COMMAND /transpose 5"), Some(LineType::Command));
        assert_eq!(classify_content("QUEUE_COMMAND /something"), Some(LineType::Command));
    }

    #[test]
    fn test_comment() {
        assert_eq!(classify_content("# just a comment"), Some(LineType::Comment));
    }

    #[test]
    fn test_track_definition() {
        assert_eq!(classify_content("c4 d4 e4"), Some(LineType::TrackDefinition));
        assert_eq!(classify_content("<harmony> g4 a4"), Some(LineType::TrackDefinition));
        assert_eq!(classify_content("14 14 26 32"), Some(LineType::TrackDefinition));
    }

    #[test]
    fn test_bare_address_command() {
        assert_eq!(classify_content("/set_bpm 120"), Some(LineType::Command));
        assert_eq!(classify_content("/free_notes"), Some(LineType::Command));
    }

    #[test]
    fn test_default_not_mistaken_for_track() {
        // "DEFAULTS" should not match DEFAULT
        assert_eq!(classify_content("DEFAULTS something"), Some(LineType::TrackDefinition));
    }

    #[test]
    fn test_sharp_in_note_classification() {
        // f#5 starts as track, not comment
        assert_eq!(classify_content("f#5 g#4"), Some(LineType::TrackDefinition));
    }

    // -- classify_source (integration) --

    #[test]
    fn test_classify_full_source() {
        let source = "\
# This is a header comment
>>> drums bass

@moogBass:bass amp0.5
c4 d4 e4 f4
g4 a4 b4 c5
# @commentedSynth
€reverb:main room0.9

COMMAND /set_bpm 120
DEFAULT amp0.3
";
        let classified = classify_source(source);

        // Total non-empty lines: 10 (after continuation joining, skipping empties)
        assert_eq!(classified.len(), 9);

        let types: Vec<LineType> = classified.iter().map(|l| l.line_type).collect();
        assert_eq!(
            types,
            vec![
                LineType::Comment,
                LineType::GroupFilter,
                LineType::SynthHeader,
                LineType::TrackDefinition,
                LineType::TrackDefinition,
                LineType::Comment,
                LineType::EffectDefinition,
                LineType::Command,
                LineType::DefaultStatement,
            ]
        );
    }

    #[test]
    fn test_classify_with_continuation() {
        let source = "\
@synth:melody
(c4 d4 e4 f4 \\
 g4 a4 b4 c5):amp0.5
";
        let classified = classify_source(source);
        assert_eq!(classified.len(), 2);
        assert_eq!(classified[0].line_type, LineType::SynthHeader);
        assert_eq!(classified[1].line_type, LineType::TrackDefinition);
        assert_eq!(
            classified[1].content,
            "(c4 d4 e4 f4  g4 a4 b4 c5):amp0.5"
        );
    }

    #[test]
    fn test_classify_inline_comment() {
        let source = "@synth # instrument header\nc4 d4 # play notes\n";
        let classified = classify_source(source);
        assert_eq!(classified.len(), 2);
        assert_eq!(classified[0].line_type, LineType::SynthHeader);
        assert_eq!(classified[0].inline_comment, Some("instrument header".to_string()));
        assert_eq!(classified[1].line_type, LineType::TrackDefinition);
        assert_eq!(classified[1].inline_comment, Some("play notes".to_string()));
    }

    #[test]
    fn test_classify_empty_lines() {
        let source = "@a\n\n\n@b\n";
        let classified = classify_source(source);
        assert_eq!(classified.len(), 2);
    }

    #[test]
    fn test_classify_comment_only_lines() {
        let source = "# one\n# two\n@synth\n";
        let classified = classify_source(source);
        assert_eq!(classified.len(), 3);
        assert_eq!(classified[0].line_type, LineType::Comment);
        assert_eq!(classified[1].line_type, LineType::Comment);
        assert_eq!(classified[2].line_type, LineType::SynthHeader);
    }

    // -- group_sections --

    fn classify(s: &str) -> Vec<ClassifiedLine> {
        classify_source(s)
    }

    #[test]
    fn test_group_empty() {
        let g = group_sections(&[]);
        assert!(g.filters.is_empty());
        assert!(g.default_statement.is_none());
        assert!(g.commands.is_empty());
        assert!(g.sections.is_empty());
    }

    #[test]
    fn test_group_single_section_with_tracks() {
        let source = "\
@moogBass:bass amp0.5
c4 d4 e4 f4
g4 a4 b4 c5
";
        let g = group_sections(&classify(source));
        assert_eq!(g.sections.len(), 1);
        assert_eq!(g.sections[0].tracks.len(), 2);
        assert!(g.sections[0].effects.is_empty());
        assert_eq!(g.sections[0].header.content, "@moogBass:bass amp0.5");
    }

    #[test]
    fn test_group_multiple_sections() {
        let source = "\
@moogBass:bass
c4 d4
@keys:melody
e4 f4
";
        let g = group_sections(&classify(source));
        assert_eq!(g.sections.len(), 2);
        assert_eq!(g.sections[0].tracks.len(), 1);
        assert_eq!(g.sections[1].tracks.len(), 1);
        assert_eq!(g.sections[0].header.content, "@moogBass:bass");
        assert_eq!(g.sections[1].header.content, "@keys:melody");
    }

    #[test]
    fn test_group_filters_first_chain() {
        let source = "\
>>> drums bass
>>> keys
# comment
@synth:drums
c4
";
        let g = group_sections(&classify(source));
        assert_eq!(g.filters.len(), 2);
        assert_eq!(g.sections.len(), 1);
    }

    #[test]
    fn test_group_filters_breaks_on_non_filter() {
        let source = "\
>>> drums
@synth
>>> keys   # this filter is past the break
";
        let g = group_sections(&classify(source));
        assert_eq!(g.filters.len(), 1);
        // Second filter becomes orphan
        assert_eq!(g.orphan_lines.len(), 1);
        assert_eq!(g.orphan_lines[0].content, ">>> keys");
    }

    #[test]
    fn test_group_default_statement() {
        let source = "\
DEFAULT amp0.5
@synth
c4
";
        let g = group_sections(&classify(source));
        assert!(g.default_statement.is_some());
        assert_eq!(g.default_statement.unwrap().content, "DEFAULT amp0.5");
    }

    #[test]
    fn test_group_last_default_wins() {
        let source = "\
DEFAULT amp0.5
@synth
DEFAULT sus1.0
";
        let g = group_sections(&classify(source));
        assert!(g.default_statement.is_some());
        assert_eq!(g.default_statement.unwrap().content, "DEFAULT sus1.0");
    }

    #[test]
    fn test_group_commands() {
        let source = "\
COMMAND /set_bpm 120
@synth
c4
COMMAND /transpose 5
";
        let g = group_sections(&classify(source));
        assert_eq!(g.commands.len(), 2);
    }

    #[test]
    fn test_group_effects_in_section() {
        let source = "\
@synth:keys
c4 e4
€reverb:main room0.9
€delay:echo time0.25
";
        let g = group_sections(&classify(source));
        assert_eq!(g.sections.len(), 1);
        assert_eq!(g.sections[0].effects.len(), 2);
        assert_eq!(g.sections[0].effects[0].content, "€reverb:main room0.9");
        assert_eq!(g.sections[0].effects[1].content, "€delay:echo time0.25");
    }

    #[test]
    fn test_group_comments_in_section() {
        let source = "\
@synth
c4 d4
# comment inside section
e4 f4
";
        let g = group_sections(&classify(source));
        assert_eq!(g.sections.len(), 1);
        // The comment is kept in the section
        assert_eq!(g.sections[0].comments.len(), 1);
        assert_eq!(g.sections[0].tracks.len(), 2);
    }

    #[test]
    fn test_group_orphan_tracks_before_header() {
        let source = "\
c4 d4
@synth
e4 f4
";
        let g = group_sections(&classify(source));
        assert_eq!(g.orphan_lines.len(), 1);
        assert_eq!(g.orphan_lines[0].content, "c4 d4");
        assert_eq!(g.sections[0].tracks.len(), 1);
    }

    // -- parse_arg_list --

    #[test]
    fn test_parse_arg_list_empty() {
        assert!(parse_arg_list("").is_empty());
        assert!(parse_arg_list("  ").is_empty());
    }

    #[test]
    fn test_parse_arg_list_simple() {
        let r = parse_arg_list("amp0.5");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0], ("amp".to_string(), "_".to_string(), "0.5".to_string()));
    }

    #[test]
    fn test_parse_arg_list_bare_value() {
        let r = parse_arg_list("0.5");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0], ("0.5".to_string(), "_".to_string(), "".to_string()));
    }

    #[test]
    fn test_parse_arg_list_keyval() {
        let r = parse_arg_list("amp=0.5,sus=1.0");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], ("amp".to_string(), "=".to_string(), "0.5".to_string()));
        assert_eq!(r[1], ("sus".to_string(), "=".to_string(), "1.0".to_string()));
    }

    #[test]
    fn test_parse_arg_list_operators() {
        let r = parse_arg_list("amp*2,sus+0.5,gain-3");
        assert_eq!(r[0], ("amp".to_string(), "*".to_string(), "2".to_string()));
        assert_eq!(r[1], ("sus".to_string(), "+".to_string(), "0.5".to_string()));
        assert_eq!(r[2], ("gain".to_string(), "-".to_string(), "3".to_string()));
    }

    // -- parse_synth_header --

    #[test]
    fn test_parse_synth_header_basic() {
        let r = parse_synth_header("@moogBass").unwrap();
        assert_eq!(r.instrument, "moogBass");
        assert!(!r.is_drone);
        assert!(!r.is_sampler);
        assert!(!r.is_selected);
        assert_eq!(r.group, None);
        assert!(r.args.is_empty());
    }

    #[test]
    fn test_parse_synth_header_with_group() {
        let r = parse_synth_header("@moogBass:bass").unwrap();
        assert_eq!(r.instrument, "moogBass");
        assert_eq!(r.group, Some("bass".to_string()));
    }

    #[test]
    fn test_parse_synth_header_with_args() {
        let r = parse_synth_header("@moogBass:bass amp0.5,sus1.0").unwrap();
        assert_eq!(r.instrument, "moogBass");
        assert_eq!(r.group, Some("bass".to_string()));
        assert!(r.args.contains(&("amp".to_string(), "0.5".to_string())));
        assert!(r.args.contains(&("sus".to_string(), "1.0".to_string())));
    }

    #[test]
    fn test_parse_synth_header_selected() {
        let r = parse_synth_header("*@SP_Roland808:drums").unwrap();
        assert!(r.is_selected);
        assert!(r.is_sampler);
        assert!(!r.is_drone);
        assert_eq!(r.instrument, "Roland808");
        assert_eq!(r.group, Some("drums".to_string()));
    }

    #[test]
    fn test_parse_synth_header_drone() {
        let r = parse_synth_header("@DR_aPad:ambient amp0.0").unwrap();
        assert!(r.is_drone);
        assert!(!r.is_sampler);
        assert_eq!(r.instrument, "aPad");
        assert_eq!(r.group, Some("ambient".to_string()));
        assert!(r.args.contains(&("amp".to_string(), "0.0".to_string())));
    }

    #[test]
    fn test_parse_synth_header_pad_config() {
        let r = parse_synth_header("*@SP_Roland808:drums 1:0 2:14 3:26").unwrap();
        assert!(r.is_sampler);
        assert!(r.is_selected);
        assert!(r.pad_config.contains(&(1, 0)));
        assert!(r.pad_config.contains(&(2, 14)));
        assert!(r.pad_config.contains(&(3, 26)));
    }

    // -- parse_track_metadata --

    #[test]
    fn test_parse_track_metadata_none() {
        assert!(parse_track_metadata("c4 d4 e4").is_none());
    }

    #[test]
    fn test_parse_track_metadata_group_only() {
        let m = parse_track_metadata("<harmony> g4 a4").unwrap();
        assert_eq!(m.group_override, Some("harmony".to_string()));
        assert!(m.arg_overrides.is_empty());
    }

    #[test]
    fn test_parse_track_metadata_with_args() {
        let m = parse_track_metadata("<harmony; amp*1.5, sus+0.3> g4 a4").unwrap();
        assert_eq!(m.group_override, Some("harmony".to_string()));
        assert_eq!(m.arg_overrides.len(), 2);
        assert_eq!(m.arg_overrides[0], ("amp".to_string(), "*".to_string(), "1.5".to_string()));
    }

    // -- parse_effect --

    #[test]
    fn test_parse_effect_basic() {
        let e = parse_effect("€reverb:main").unwrap();
        assert_eq!(e.effect_type, "reverb");
        assert_eq!(e.id, "main");
        assert!(e.args.is_empty());
    }

    #[test]
    fn test_parse_effect_with_args() {
        let e = parse_effect("€reverb:main room0.9,mix0.5").unwrap();
        assert_eq!(e.effect_type, "reverb");
        assert_eq!(e.id, "main");
        assert!(e.args.contains(&("room".to_string(), "0.9".to_string())));
        assert!(e.args.contains(&("mix".to_string(), "0.5".to_string())));
    }

    // -- parse_command --

    #[test]
    fn test_parse_command_basic() {
        let c = parse_command("COMMAND /set_bpm 120").unwrap();
        assert_eq!(c.context, CommandContext::All);
        assert_eq!(c.address, "/set_bpm");
        assert_eq!(c.args, vec!["120"]);
    }

    #[test]
    fn test_parse_command_update() {
        let c = parse_command("UPDATE_COMMAND /transpose 5").unwrap();
        assert_eq!(c.context, CommandContext::Update);
        assert_eq!(c.address, "/transpose");
        assert_eq!(c.args, vec!["5"]);
    }

    #[test]
    fn test_parse_command_queue() {
        let c = parse_command("QUEUE_COMMAND /something").unwrap();
        assert_eq!(c.context, CommandContext::Queue);
        assert_eq!(c.address, "/something");
        assert!(c.args.is_empty());
    }

    #[test]
    fn test_parse_command_bare_address() {
        let c = parse_command("/set_bpm 120").unwrap();
        assert_eq!(c.context, CommandContext::All);
        assert_eq!(c.address, "/set_bpm");
        assert_eq!(c.args, vec!["120"]);
    }

    // -- parse_filter --

    #[test]
    fn test_parse_filter_basic() {
        let f = parse_filter(">>> drums bass keys").unwrap();
        assert_eq!(f.groups, vec!["drums", "bass", "keys"]);
    }

    #[test]
    fn test_parse_filter_empty() {
        let f = parse_filter(">>>").unwrap();
        assert!(f.groups.is_empty());
    }

    // -- Integration: parse from GroupedBillboard --

    #[test]
    fn test_parse_integration() {
        let source = "\
# header
>>> drums bass

COMMAND /set_bpm 120
DEFAULT amp0.5

*@SP_Roland808:drums ofs0 1:0 2:14
14 14 26 32
€reverb:main room0.9

@moogBass:bass
<harmony; amp*1.5> c4 d4
";
        let classified = classify_source(source);
        let grouped = group_sections(&classified);

        // Filters
        let filters = parse_all_filters(&grouped);
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].as_ref().unwrap().groups, vec!["drums", "bass"]);

        // Commands
        let cmds = parse_all_commands(&grouped);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].as_ref().unwrap().address, "/set_bpm");

        // Section 0: sampler
        let h0 = parse_synth_header(&grouped.sections[0].header.content).unwrap();
        assert!(h0.is_sampler);
        assert!(h0.is_selected);
        assert_eq!(h0.instrument, "Roland808");

        let eff0 = parse_section_effects(&grouped.sections[0]);
        assert_eq!(eff0.len(), 1);
        assert_eq!(eff0[0].as_ref().unwrap().effect_type, "reverb");

        // Section 1: bass
        let h1 = parse_synth_header(&grouped.sections[1].header.content).unwrap();
        assert_eq!(h1.instrument, "moogBass");
        assert_eq!(h1.group, Some("bass".to_string()));

        let meta = parse_track_metadata(&grouped.sections[1].tracks[0].content);
        assert!(meta.is_some());
        assert_eq!(meta.unwrap().group_override, Some("harmony".to_string()));
    }

    #[test]
    fn test_group_complex_file() {
        let source = "\
# header comment
>>> drums bass

@SP_Roland808:drums ofs0
14 14 26 32
€reverb:main room0.5

@moogBass:bass amp0.5
c4 d4
# commented track
g4 a4

COMMAND /set_bpm 120
DEFAULT amp0.3
";
        let g = group_sections(&classify(source));
        assert_eq!(g.filters.len(), 1);
        assert_eq!(g.commands.len(), 1);
        assert!(g.default_statement.is_some());
        assert_eq!(g.sections.len(), 2);
        assert_eq!(g.sections[0].tracks.len(), 1);
        assert_eq!(g.sections[0].effects.len(), 1);
        assert_eq!(g.sections[1].tracks.len(), 2);
        assert_eq!(g.sections[1].comments.len(), 1);
    }

    // -- resolve_args --

    #[test]
    fn test_resolve_args_empty() {
        let r = resolve_args(&HashMap::new(), &HashMap::new(), &HashMap::new());
        assert!(r.is_empty());
    }

    #[test]
    fn test_resolve_args_defaults_only() {
        let mut d = HashMap::new();
        d.insert("amp".to_string(), 0.5);
        let r = resolve_args(&d, &HashMap::new(), &HashMap::new());
        assert_eq!(r.get("amp"), Some(&0.5));
    }

    #[test]
    fn test_resolve_args_header_overrides_default() {
        let mut d = HashMap::new();
        d.insert("amp".to_string(), 0.5);
        let mut h = HashMap::new();
        h.insert("amp".to_string(), 1.0);
        let r = resolve_args(&d, &h, &HashMap::new());
        assert_eq!(r.get("amp"), Some(&1.0));
    }

    #[test]
    fn test_resolve_args_override_add() {
        let mut d = HashMap::new();
        d.insert("amp".to_string(), 0.5);
        let mut t = HashMap::new();
        t.insert("amp".to_string(), ('+', 0.3));
        let r = resolve_args(&d, &HashMap::new(), &t);
        assert!((r.get("amp").unwrap() - 0.8).abs() < 1e-10);
    }

    #[test]
    fn test_resolve_args_override_mult() {
        let mut d = HashMap::new();
        d.insert("amp".to_string(), 0.5);
        let mut t = HashMap::new();
        t.insert("amp".to_string(), ('*', 2.0));
        let r = resolve_args(&d, &HashMap::new(), &t);
        assert!((r.get("amp").unwrap() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_resolve_args_override_sub() {
        let mut h = HashMap::new();
        h.insert("sus".to_string(), 1.0);
        let mut t = HashMap::new();
        t.insert("sus".to_string(), ('-', 0.3));
        let r = resolve_args(&HashMap::new(), &h, &t);
        assert!((r.get("sus").unwrap() - 0.7).abs() < 1e-10);
    }

    // -- build_billboard / parse --

    #[test]
    fn test_parse_empty() {
        let b = parse("");
        assert!(b.sections.is_empty());
        assert!(b.filters.is_empty());
        assert!(b.commands.is_empty());
        assert!(b.default_args.is_empty());
    }

    #[test]
    fn test_parse_default_args() {
        let b = parse("DEFAULT amp0.5,sus1.0");
        assert_eq!(b.default_args.get("amp"), Some(&0.5));
        assert_eq!(b.default_args.get("sus"), Some(&1.0));
    }

    #[test]
    fn test_parse_synths_and_tracks() {
        let source = "\
@moogBass:bass amp0.5
c4 d4 e4
g4 a4 b4
";
        let b = parse(source);
        assert_eq!(b.sections.len(), 1);
        assert_eq!(b.sections[0].header.instrument, "moogBass");
        assert_eq!(b.sections[0].header.group, Some("bass".to_string()));
        assert_eq!(b.sections[0].header.default_args.get("amp"), Some(&0.5));
        assert_eq!(b.sections[0].tracks.len(), 2);
        assert_eq!(b.sections[0].tracks[0].content, "c4 d4 e4");
        assert_eq!(b.sections[0].tracks[0].index, 0);
        assert!(b.sections[0].tracks[0].enabled);
    }

    #[test]
    fn test_parse_complex() {
        let source = "\
>>> drums bass

COMMAND /set_bpm 120
DEFAULT amp0.3

@SP_Roland808:drums ofs0 1:0 2:14
14 14 26 32
€reverb:main room0.9,mix0.5

@moogBass:bass
<harmony; amp*1.5> c4 d4
";
        let b = parse(source);

        // Filters
        assert_eq!(b.filters.len(), 1);
        assert_eq!(b.filters[0], vec!["drums", "bass"]);

        // Commands
        assert_eq!(b.commands.len(), 1);
        assert_eq!(b.commands[0].address, "/set_bpm");

        // Default args
        assert_eq!(b.default_args.get("amp"), Some(&0.3));

        // Section 0: sampler
        let s0 = &b.sections[0];
        assert!(s0.header.is_sampler);
        assert!(!s0.header.is_selected);
        assert_eq!(s0.header.instrument, "Roland808");
        assert_eq!(s0.header.pad_config, vec![(1, 0), (2, 14)]);
        assert_eq!(s0.tracks.len(), 1);
        assert_eq!(s0.tracks[0].content, "14 14 26 32");
        assert_eq!(s0.effects.len(), 1);
        assert_eq!(s0.effects[0].effect_type, "reverb");
        assert_eq!(s0.effects[0].args.get("room"), Some(&0.9));

        // Section 1: bass
        let s1 = &b.sections[1];
        assert_eq!(s1.header.instrument, "moogBass");
        assert_eq!(s1.tracks.len(), 1);
        assert_eq!(s1.tracks[0].group_override, Some("harmony".to_string()));
        assert_eq!(s1.tracks[0].content, "c4 d4");
        // Resolve args for track: default(amp=0.3) + header(none) + override(amp*1.5)
        let resolved = resolve_args(
            &b.default_args,
            &s1.header.default_args,
            &s1.tracks[0].arg_overrides,
        );
        assert!((resolved.get("amp").unwrap() - 0.45).abs() < 1e-10);
    }

    #[test]
    fn test_parse_track_metadata_stripped() {
        let source = "\
@synth
<harmony; amp=2.0> c4 d4
";
        let b = parse(source);
        assert_eq!(b.sections[0].tracks[0].content, "c4 d4");
        assert_eq!(b.sections[0].tracks[0].group_override, Some("harmony".to_string()));
        let overrides = &b.sections[0].tracks[0].arg_overrides;
        assert_eq!(overrides.get("amp"), Some(&('=', 2.0)));
    }
}
