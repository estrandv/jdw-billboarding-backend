/// Full Billboard Notation parser.
///
/// Stages (per PLAN_full_billboard_parser.md):
///   Stage 1 — Line classifier + continuation + inline comments
///   Stage 2 — Section grouper                                    ← current
///   Stage 3 — Low-level parsers
///   Stage 4 — Billboard construction + argument inheritance
///   Stage 5 — OSC conversion
///   Stage 6 — jdw-suite integration
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
}
