const SONGS_DIR: &str = "/home/estrandv/programming/jdw-pycompose/songs/";

#[test]
fn test_all_bbd_files_parse() {
    let dir = std::fs::read_dir(SONGS_DIR).unwrap();
    let mut parsed = 0;
    let mut errors = Vec::new();

    for entry in dir {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().map(|e| e == "bbd").unwrap_or(false) {
            let billboard = jdw_billboarding_backend::parse_billboard_file(path.to_str().unwrap());
            match billboard {
                Ok(b) => {
                    parsed += 1;
                    eprintln!("OK: {} ({} sections, {} commands)",
                        path.file_name().unwrap().to_string_lossy(),
                        b.sections.len(),
                        b.commands.len());
                }
                Err(e) => {
                    errors.push((path.file_name().unwrap().to_string_lossy().to_string(), e));
                }
            }
        }
    }

    if !errors.is_empty() {
        for (name, err) in &errors {
            eprintln!("FAIL: {}: {}", name, err);
        }
        panic!("{} of {} files failed to parse", errors.len(), parsed + errors.len());
    }

    eprintln!("All {} .bbd files parsed successfully.", parsed);
}

#[test]
fn test_gong_bbd_parses() {
    let gong_path = "/home/estrandv/programming/jdw-pycompose/songs/gong.bbd";
    let content = std::fs::read_to_string(gong_path).unwrap();
    let billboard = jdw_billboarding_backend::parse_billboard(&content);

    // SP_/DR_ prefixes are stripped from instrument name, tracked as flags
    let synth_names: Vec<&str> = billboard.sections.iter()
        .map(|s| s.header.instrument.as_str())
        .collect();
    eprintln!("synth names: {:?}", synth_names);
    assert!(synth_names.contains(&"EMU_SP12"), "expected EMU_SP12, got {:?}", synth_names);
    assert!(synth_names.contains(&"Roland808"), "expected Roland808");
    assert!(synth_names.contains(&"cheapPiano"), "expected cheapPiano");
    assert!(synth_names.contains(&"eighties"), "expected eighties");
    assert!(synth_names.contains(&"FMRhodes"), "expected FMRhodes");
}

#[test]
fn test_gong_bbd_via_file_api() {
    let gong_path = "/home/estrandv/programming/jdw-pycompose/songs/gong.bbd";
    let billboard = jdw_billboarding_backend::parse_billboard_file(gong_path).unwrap();

    let synth_count = billboard.sections.len();
    assert!(synth_count >= 15, "expected many synths, got {}", synth_count);

    // Should have commands
    assert!(!billboard.commands.is_empty(), "expected commands");
}

#[test]
fn test_trumpets_bbd_with_macros() {
    let trumpets_path = "/home/estrandv/programming/jdw-pycompose/songs/trumpets.bbd";
    let billboard = jdw_billboarding_backend::parse_billboard_file(trumpets_path).unwrap();

    let synth_names: Vec<&str> = billboard.sections.iter()
        .map(|s| s.header.instrument.as_str())
        .collect();
    eprintln!("trumpets synths: {:?}", synth_names);

    // The $chug macro is used and should expand correctly
    assert!(synth_names.contains(&"Roland808"), "expected Roland808");
    assert!(synth_names.contains(&"trumpet"), "expected trumpet");

    // Verify tracks have shuttle notation content (macros should expand)
    for section in &billboard.sections {
        for track in &section.tracks {
            assert!(!track.content.trim().is_empty(),
                "track #{} in {} has empty content after macro expansion",
                track.index, section.header.instrument);
        }
    }
}

#[test]
fn test_trumpets_bbd_raw_fails_without_macros() {
    // Without macro expansion, $chug references should remain unexpanded
    // and will fail shuttle parsing later, but the billboard still parses
    let trumpets_path = "/home/estrandv/programming/jdw-pycompose/songs/trumpets.bbd";
    let content = std::fs::read_to_string(trumpets_path).unwrap();

    let billboard = jdw_billboarding_backend::parse_billboard(&content);

    // Tracks with $macro calls will have their raw content preserved
    let macro_tracks: Vec<&str> = billboard.sections.iter()
        .flat_map(|s| s.tracks.iter())
        .filter(|t| t.content.contains('$'))
        .map(|t| t.content.as_str())
        .collect();

    if !macro_tracks.is_empty() {
        eprintln!("Tracks with unexpanded $macros: {:?}", macro_tracks);
    }

    assert!(!macro_tracks.is_empty(),
        "expected some tracks with unexpanded $macros in raw parse");
}
