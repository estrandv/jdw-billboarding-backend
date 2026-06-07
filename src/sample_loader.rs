/// Sample loading — port of jdw-pycompose's file_utilities.py.
///
/// Iterates sample pack directories, assigns buffer_index (global, 100+)
/// and tone_index (per-pack, 0+), categorizes by filename keyword,
/// and builds `/load_sample` OSC messages.
use std::path::Path;

/// A sample file discovered during scanning.
#[derive(Debug, Clone)]
pub struct Sample {
    pub path: String,
    pub sample_pack: String,
    pub buffer_index: u32,
    pub category: String,
    pub tone_index: u32,
}

/// A sample ready to send via `/load_sample`.
#[derive(Debug, Clone)]
pub struct SampleLoadMessage {
    pub sample: Sample,
    pub osc_args: Vec<rosc::OscType>,
}

/// Allowed file extensions for sample files.
const SAMPLE_EXTENSIONS: &[&str] = &["wav"];

/// Buffer index starts at 100 to leave room for SuperCollider internal buffers.
const FIRST_BUFFER_INDEX: u32 = 100;

impl Sample {
    /// Build OSC args for `/load_sample`: [path, pack, buffer_index, category, tone_index]
    pub fn as_osc_args(&self) -> Vec<rosc::OscType> {
        vec![
            rosc::OscType::String(self.path.clone()),
            rosc::OscType::String(self.sample_pack.clone()),
            rosc::OscType::Int(self.buffer_index as i32),
            rosc::OscType::String(self.category.clone()),
            rosc::OscType::Int(self.tone_index as i32),
        ]
    }
}

/// Scan a root directory for sample packs, returning all discovered samples.
///
/// Each subdirectory is treated as a sample pack. Files with extensions in
/// SAMPLE_EXTENSIONS are collected, natsorted, and assigned indices.
pub fn read_sample_packs(samples_root: &str) -> Vec<Sample> {
    let root = expand_tilde(samples_root);
    let root_path = Path::new(&root);
    if !root_path.is_dir() {
        return Vec::new();
    }

    let mut all_samples = Vec::new();
    let mut buffer_index = FIRST_BUFFER_INDEX;

    // Collect and sort pack directories
    let mut pack_dirs: Vec<_> = match std::fs::read_dir(root_path) {
        Ok(rd) => rd.filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .collect(),
        Err(_) => return Vec::new(),
    };
    pack_dirs.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());

    for pack_entry in &pack_dirs {
        let pack_name = pack_entry.file_name().to_string_lossy().to_string();
        if pack_name.starts_with('.') {
            continue; // skip hidden dirs like .git
        }
        let pack_path = pack_entry.path();

        // Collect wav files, filter by extension
        let mut files: Vec<_> = match std::fs::read_dir(&pack_path) {
            Ok(rd) => rd.filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    SAMPLE_EXTENSIONS.iter().any(|ext| name.ends_with(&format!(".{}", ext)))
                })
                .collect(),
            Err(_) => continue,
        };
        // Case-insensitive alphabetical sort by filename
        files.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());

        let mut tone_index: u32 = 0;
        for file_entry in &files {
            let file_name = file_entry.file_name().to_string_lossy().to_string();
            let category = get_sample_category(&file_name);
            all_samples.push(Sample {
                path: file_entry.path().to_string_lossy().to_string(),
                sample_pack: pack_name.clone(),
                buffer_index,
                category,
                tone_index,
            });
            buffer_index += 1;
            tone_index += 1;
        }
    }

    all_samples
}

/// Simple natural sort key: lowercase string with number extraction for ordering.
/// Simple tilde expansion: ~ → $HOME, no support for ~user.
fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return home + &path[1..];
        }
    }
    path.to_string()
}

/// Categorize a sample file by keyword matching in the filename.
///
/// Matches are case-insensitive. Returns a short category tag like "bd", "sn",
/// "hh", etc., or empty string if no keywords match.
fn get_sample_category(file_name: &str) -> String {
    let lower = file_name.to_lowercase();
    let matchers: Vec<(&str, Vec<&str>)> = vec![
        ("bd", vec!["bd"]),
        ("hh", vec!["hh", "hat", "ride"]),
        ("cy", vec!["cy", "crash", "cr"]),
        ("sn", vec!["sn", "sd"]),
        ("be", vec!["cb", "bell"]),
        ("to", vec!["lt", "ht", "tom", "mc", "lb", "to"]),
        ("sh", vec!["mc", "ma", "sh"]),
        ("fx", vec!["fx"]),
        ("st", vec!["st"]),
    ];

    for (category, keywords) in &matchers {
        for kw in keywords {
            if lower.contains(kw) {
                return category.to_string();
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_category_bass_drum() {
        assert_eq!(get_sample_category("BD7500.WAV"), "bd");
        assert_eq!(get_sample_category("bd_01.wav"), "bd");
        assert_eq!(get_sample_category("Kick_bd.wav"), "bd");
    }

    #[test]
    fn test_category_snare() {
        assert_eq!(get_sample_category("SD0000.WAV"), "sn");
        assert_eq!(get_sample_category("snare_01.wav"), "sn");
    }

    #[test]
    fn test_category_hihat() {
        assert_eq!(get_sample_category("HH_01.wav"), "hh");
        assert_eq!(get_sample_category("hat_closed.wav"), "hh");
        assert_eq!(get_sample_category("ride_bell.wav"), "hh");
    }

    #[test]
    fn test_category_cymbal() {
        assert_eq!(get_sample_category("CY0000.WAV"), "cy");
        assert_eq!(get_sample_category("crash_01.wav"), "cy");
    }

    #[test]
    fn test_category_tom() {
        assert_eq!(get_sample_category("LT_01.wav"), "to");
        assert_eq!(get_sample_category("tom_high.wav"), "to");
        assert_eq!(get_sample_category("HT0000.WAV"), "to");
    }

    #[test]
    fn test_category_bell() {
        assert_eq!(get_sample_category("CB-01.wav"), "be");
        assert_eq!(get_sample_category("cowbell.wav"), "be");
    }

    #[test]
    fn test_category_unknown() {
        assert_eq!(get_sample_category("unknown_sound.wav"), "");
        assert_eq!(get_sample_category("xyz.wav"), "");
    }

    #[test]
    fn test_read_sample_packs_empty_dir() {
        let tmp = std::env::temp_dir().join("jdw_test_empty_samples");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let samples = read_sample_packs(&tmp.to_string_lossy());
        assert!(samples.is_empty());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_read_sample_packs_basic() {
        let tmp = std::env::temp_dir().join("jdw_test_samples");
        let _ = fs::remove_dir_all(&tmp);
        let pack_dir = tmp.join("TestPack");
        fs::create_dir_all(&pack_dir).unwrap();

        // Create some fake wav files
        fs::write(pack_dir.join("BD_01.wav"), b"fake").unwrap();
        fs::write(pack_dir.join("SD_01.wav"), b"fake").unwrap();
        fs::write(pack_dir.join("notes.txt"), b"not a sample").unwrap(); // should be skipped

        let samples = read_sample_packs(&tmp.to_string_lossy());
        assert_eq!(samples.len(), 2);

        let bd = &samples[0];
        assert_eq!(bd.sample_pack, "TestPack");
        assert_eq!(bd.category, "bd");
        assert_eq!(bd.tone_index, 0);
        assert!(bd.buffer_index >= 100);

        let sd = &samples[1];
        assert_eq!(sd.sample_pack, "TestPack");
        assert_eq!(sd.category, "sn");
        assert_eq!(sd.tone_index, 1);
        assert_eq!(sd.buffer_index, bd.buffer_index + 1, "buffer_index should increment globally");

        // Check OSC args
        let bd_args = bd.as_osc_args();
        assert_eq!(bd_args.len(), 5);
        assert!(matches!(&bd_args[0], rosc::OscType::String(p) if p.contains("BD_01.wav")));
        assert_eq!(bd_args[1], rosc::OscType::String("TestPack".to_string()));
        assert!(matches!(bd_args[2], rosc::OscType::Int(_)));
        assert_eq!(bd_args[3], rosc::OscType::String("bd".to_string()));
        assert_eq!(bd_args[4], rosc::OscType::Int(0));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_buffer_index_starts_at_100() {
        let tmp = std::env::temp_dir().join("jdw_test_buf_idx");
        let _ = fs::remove_dir_all(&tmp);
        let pack_dir = tmp.join("P");
        fs::create_dir_all(&pack_dir).unwrap();
        fs::write(pack_dir.join("test.wav"), b"fake").unwrap();

        let samples = read_sample_packs(&tmp.to_string_lossy());
        assert_eq!(samples[0].buffer_index, 100);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_get_default_samples_builds_messages() {
        let tmp = std::env::temp_dir().join("jdw_test_def_samples");
        let _ = fs::remove_dir_all(&tmp);
        let pack_dir = tmp.join("P");
        fs::create_dir_all(&pack_dir).unwrap();
        fs::write(pack_dir.join("BD.wav"), b"fake").unwrap();

        let msgs = get_default_samples(&tmp.to_string_lossy());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].osc_args.len(), 5);
        // /load_sample args: [path, pack, buffer, category, tone_index]
        assert!(matches!(&msgs[0].osc_args[0], rosc::OscType::String(p) if p.ends_with("BD.wav")));
        assert_eq!(msgs[0].osc_args[2], rosc::OscType::Int(100));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_skip_hidden_dirs() {
        let tmp = std::env::temp_dir().join("jdw_test_hidden");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join(".git")).unwrap();
        fs::write(tmp.join(".git").join("test.wav"), b"fake").unwrap();
        let pack_dir = tmp.join("RealPack");
        fs::create_dir_all(&pack_dir).unwrap();
        fs::write(pack_dir.join("real.wav"), b"fake").unwrap();

        let samples = read_sample_packs(&tmp.to_string_lossy());
        assert_eq!(samples.len(), 1, "should skip hidden dirs like .git");
        assert_eq!(samples[0].sample_pack, "RealPack");

        let _ = fs::remove_dir_all(&tmp);
    }
}

/// Build `SampleLoadMessage`s for all discovered samples, with pre-built OSC args.
pub fn get_default_samples(samples_root: &str) -> Vec<SampleLoadMessage> {
    read_sample_packs(samples_root)
        .into_iter()
        .map(|sample| {
            let osc_args = sample.as_osc_args();
            SampleLoadMessage { sample, osc_args }
        })
        .collect()
}
