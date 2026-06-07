/// Note/music theory utilities for shuttle notation → MIDI frequency conversion.

/// MIDI note numbers for letter names (c=0, c#/db=1, ..., b=11).
pub fn note_letter_to_midi(note: &str) -> i32 {
    match note {
        "c" => 0, "c#" | "db" => 1,
        "d" => 2, "d#" | "eb" => 3,
        "e" => 4,
        "f" => 5, "f#" | "gb" => 6,
        "g" => 7, "g#" | "ab" => 8,
        "a" => 9, "a#" | "bb" => 10,
        "b" => 11,
        _ => -1,
    }
}

/// Scale interval patterns (distance in semitones from root).
fn scale_intervals(scale_type: &str) -> &[i32] {
    match scale_type {
        "min" => &[2, 1, 2, 2, 1, 2],
        _ => &[2, 2, 1, 2, 2, 2], // maj
    }
}

/// Generate a scale as a sorted list of chromatic MIDI note numbers.
pub fn generate_scale(root_note: i32, scale_type: &str) -> Vec<i32> {
    let intervals = scale_intervals(scale_type);
    let mut raw = vec![root_note];
    let mut step = root_note;
    for d in intervals {
        step += d;
        raw.push(step);
    }

    // Wrap into [0, 11] chromatic range, deduplicate, sort
    let mut chroma: Vec<i32> = raw.iter().map(|i| i % 12).collect();
    chroma.sort_unstable();
    chroma.dedup();
    chroma
}

/// Fetch element at `raw_index`, wrapping around the list length.
pub fn get_in_list(raw_index: i32, list: &[i32]) -> i32 {
    if list.is_empty() {
        return 0;
    }
    let indices = (list.len() - 1) as i32;
    if indices == 0 {
        return list[0];
    }
    let times = raw_index as f64 / indices as f64;
    let subtract = if times > 1.0 {
        (indices * (times as i32)) + 1
    } else {
        0
    };
    let re_attempt = raw_index - subtract;
    let idx = re_attempt.max(0).min(indices) as usize;
    list[idx]
}

/// Resolve a shuttle note index against a scale to a MIDI note number.
///
/// `note_id` is the numeric index in the shuttle notation (e.g., `0`, `1`, `23`).
/// `scale_root_letter` is the key (e.g., `"c"`, `"g"`).
/// `scale_type_key` is `"maj"` or `"min"`.
pub fn resolve_index(note_id: i32, scale_root_letter: &str, scale_type_key: &str) -> i32 {
    let root_note = note_letter_to_midi(scale_root_letter);
    let root = if root_note < 0 { 0 } else { root_note };
    let my_scale = generate_scale(root, scale_type_key);
    let scale_indices = (my_scale.len() - 1) as i32;
    let raw_scaled = get_in_list(note_id, &my_scale);
    let added_octaves = if scale_indices > 0 && (note_id as f64 / scale_indices as f64) > 1.0 {
        note_id / scale_indices
    } else {
        0
    };
    raw_scaled + 12 * added_octaves
}

/// Convert a MIDI note number to frequency in Hz.
pub fn midi_to_hz(note: f64) -> f64 {
    440.0 * 2.0_f64.powf((note - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_note_letter_to_midi() {
        assert_eq!(note_letter_to_midi("c"), 0);
        assert_eq!(note_letter_to_midi("c#"), 1);
        assert_eq!(note_letter_to_midi("db"), 1);
        assert_eq!(note_letter_to_midi("e"), 4);
        assert_eq!(note_letter_to_midi("g"), 7);
        assert_eq!(note_letter_to_midi("b"), 11);
        assert_eq!(note_letter_to_midi("x"), -1);
    }

    #[test]
    fn test_generate_c_maj() {
        let scale = generate_scale(0, "maj");
        assert_eq!(scale, vec![0, 2, 4, 5, 7, 9, 11]);
    }

    #[test]
    fn test_generate_c_min() {
        let scale = generate_scale(0, "min");
        assert_eq!(scale, vec![0, 2, 3, 5, 7, 8, 10]);
    }

    #[test]
    fn test_generate_g_maj() {
        let scale = generate_scale(7, "maj");
        // G maj: G(7) A(9) B(11) C(0) D(2) E(4) F#(6)
        assert_eq!(scale, vec![0, 2, 4, 6, 7, 9, 11]);
    }

    #[test]
    fn test_resolve_index_c_maj() {
        let resolved: Vec<i32> = (0..8).map(|i| resolve_index(i, "c", "maj")).collect();
        assert_eq!(resolved, vec![0, 2, 4, 5, 7, 9, 11, 12]);
    }

    #[test]
    fn test_midi_to_hz() {
        let a4 = midi_to_hz(69.0);
        assert!((a4 - 440.0).abs() < 0.01);
        let c4 = midi_to_hz(60.0);
        assert!((c4 - 261.63).abs() < 0.1);
    }

    #[test]
    fn test_get_in_list() {
        let list = vec![0, 2, 4, 5, 7, 9, 11];
        assert_eq!(get_in_list(0, &list), 0);
        assert_eq!(get_in_list(3, &list), 5);
        // get_in_list wraps around; octave boost handled by resolve_index
        assert_eq!(get_in_list(7, &list), 0);
    }
}
