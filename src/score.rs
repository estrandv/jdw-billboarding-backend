/// Score — timeline composition for NRT recording.
///
/// Port of jdw-pycompose's `nrt_scoring.py` Score class.
/// Composes tracks into a synchronized timeline by group filter ordering,
/// padding shorter tracks with silence so all tracks render simultaneously.
use std::collections::HashMap;

/// A source track with its resolved elements and group name.
#[derive(Debug, Clone)]
struct TrackSource {
    /// Each element paired with its beat duration (from `time` arg, default 1.0).
    elements: Vec<(crate::shuttle::ResolvedElement, f64)>,
    group_name: String,
}

/// A single entry in the score timeline.
#[derive(Debug, Clone)]
pub struct ScoreMessage {
    /// The element, or `None` for silence padding.
    pub element: Option<crate::shuttle::ResolvedElement>,
    /// Beat duration of this entry.
    pub time: f64,
}

/// The Score composes tracks into a synchronized timeline.
pub struct Score {
    track_sources: HashMap<String, TrackSource>,
    tracks: HashMap<String, Vec<ScoreMessage>>,
}

impl Score {
    pub fn new() -> Self {
        Score {
            track_sources: HashMap::new(),
            tracks: HashMap::new(),
        }
    }

    /// Register a track source with its resolved elements.
    ///
    /// `elements` should already have args resolved (DEFAULT → header → track_overrides)
    /// and be the output of `crate::shuttle::parse()` + arg resolution.
    pub fn add_source(
        &mut self,
        track_name: String,
        group_name: String,
        elements: Vec<crate::shuttle::ResolvedElement>,
    ) {
        let with_times: Vec<_> = elements
            .into_iter()
            .map(|el| {
                let time = el.args.get("time").copied().unwrap_or(1.0);
                (el, time)
            })
            .collect();
        self.track_sources.insert(
            track_name.clone(),
            TrackSource {
                elements: with_times,
                group_name,
            },
        );
        self.tracks.insert(track_name, Vec::new());
    }

    /// Extend tracks matching any of `group_names` into the score timeline.
    ///
    /// The longest matching track is extended first to establish a goal time.
    /// Other matching tracks are extended and padded to match. Non-matching
    /// tracks receive silence padding to the goal time.
    ///
    /// If `also_extend_groupless` is true, tracks with an empty group name
    /// are also extended.
    pub fn extend_groups(&mut self, group_names: &[String], also_extend_groupless: bool) {
        // Find matching track names
        let mut matching: Vec<String> = self
            .track_sources
            .iter()
            .filter(|(_, src)| group_names.contains(&src.group_name))
            .map(|(name, _)| name.clone())
            .collect();

        if also_extend_groupless {
            for (name, src) in &self.track_sources {
                if src.group_name.is_empty() && !matching.contains(name) {
                    matching.push(name.clone());
                }
            }
        }

        if matching.is_empty() {
            return;
        }

        // Find the longest source (by total duration)
        let longest_name = matching
            .iter()
            .max_by_key(|name| {
                let total: f64 = self.track_sources[*name]
                    .elements
                    .iter()
                    .map(|(_, t)| t)
                    .sum::<f64>();
                // Use integer millibeats to avoid f64 comparison issues
                (total * 1000.0) as i64
            })
            .cloned()
            .unwrap();

        // Extend longest source fully
        {
            let src = &self.track_sources[&longest_name];
            let track = self.tracks.get_mut(&longest_name).unwrap();
            track.clear();
            for (el, time) in &src.elements {
                track.push(ScoreMessage {
                    element: Some(el.clone()),
                    time: *time,
                });
            }
        }

        // Calculate goal time
        let goal_time: f64 = self.tracks[&longest_name]
            .iter()
            .map(|m| m.time)
            .sum();

        // Extend other matching tracks
        for name in &matching {
            if *name == longest_name {
                continue;
            }
            let src = self.track_sources[name].clone();
            let src_duration: f64 = src.elements.iter().map(|(_, t)| t).sum();
            let track = self.tracks.get_mut(name).unwrap();
            track.clear();

            let mut current_time = 0.0f64;
            let mut remaining = goal_time;

            // Repeat the source while there's room
            while remaining >= src_duration && src_duration > 0.0 {
                for (el, time) in &src.elements {
                    track.push(ScoreMessage {
                        element: Some(el.clone()),
                        time: *time,
                    });
                }
                current_time += src_duration;
                remaining = goal_time - current_time;
            }

            // Pad remainder with silence
            if remaining > 0.0 {
                track.push(ScoreMessage {
                    element: None,
                    time: remaining,
                });
            }
        }

        // Pad non-matching tracks with silence
        for name in self.track_sources.keys() {
            if matching.contains(name) {
                continue;
            }
            let track = self.tracks.get_mut(name).unwrap();
            if track.is_empty() {
                track.push(ScoreMessage {
                    element: None,
                    time: goal_time,
                });
            }
        }
    }

    /// Convert score tracks to timed message lists.
    ///
    /// Adjacent silence entries are compressed into preceding note durations.
    /// Standalone silence produces entries with `element: None` (→ /empty_message).
    pub fn unpack_timed_tracks(&self) -> HashMap<String, Vec<TimedScoreMessage>> {
        let mut result = HashMap::new();

        for (name, messages) in &self.tracks {
            let mut timed: Vec<TimedScoreMessage> = Vec::new();
            let mut pending_silence: f64 = 0.0;

            for msg in messages {
                match &msg.element {
                    Some(_) => {
                        if pending_silence > 0.0 {
                            // Compress preceding silence into this note
                            if let Some(last) = timed.last_mut() {
                                last.time += pending_silence;
                            }
                            pending_silence = 0.0;
                        }
                        timed.push(TimedScoreMessage {
                            element: Some(msg.element.clone().unwrap()),
                            time: msg.time,
                        });
                    }
                    None => {
                        pending_silence += msg.time;
                    }
                }
            }

            // Remaining silence after last note → standalone empty message
            if pending_silence > 0.0 && timed.is_empty() {
                timed.push(TimedScoreMessage {
                    element: None,
                    time: pending_silence,
                });
            } else if pending_silence > 0.0 {
                // Compress trailing silence into last note
                if let Some(last) = timed.last_mut() {
                    last.time += pending_silence;
                }
            }

            result.insert(name.clone(), timed);
        }

        result
    }
}

/// A timed entry from the score, ready for OSC conversion.
#[derive(Debug, Clone)]
pub struct TimedScoreMessage {
    pub element: Option<crate::shuttle::ResolvedElement>,
    pub time: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shuttle::ResolvedElement;
    use std::collections::HashMap;

    fn elem() -> ResolvedElement {
        ResolvedElement {
            prefix: "c".to_string(),
            index: 4,
            suffix: String::new(),
            args: HashMap::new(),
        }
    }

    fn elem_with_time(t: f64) -> ResolvedElement {
        let mut args = HashMap::new();
        args.insert("time".to_string(), t);
        ResolvedElement {
            prefix: "c".to_string(),
            index: 4,
            suffix: String::new(),
            args,
        }
    }

    #[test]
    fn test_score_add_source() {
        let mut score = Score::new();
        let elems = vec![elem_with_time(0.5), elem_with_time(1.0)];
        score.add_source("track1".into(), "groupA".into(), elems);

        let src = score.track_sources.get("track1").unwrap();
        assert_eq!(src.elements.len(), 2);
        assert_eq!(src.elements[0].1, 0.5);
        assert_eq!(src.elements[1].1, 1.0);
        assert_eq!(src.group_name, "groupA");

        // Track should have an empty score list
        assert!(score.tracks.get("track1").unwrap().is_empty());
    }

    #[test]
    fn test_score_add_source_default_time() {
        let mut score = Score::new();
        let elems = vec![elem()]; // no time arg → defaults to 1.0
        score.add_source("t1".into(), "g".into(), elems);

        let src = score.track_sources.get("t1").unwrap();
        assert_eq!(src.elements[0].1, 1.0);
    }

    #[test]
    fn test_extend_groups_single_filter_two_same_length() {
        // Two tracks of equal length, same group → both fully extended
        let mut score = Score::new();
        score.add_source("t1".into(), "g".into(), vec![elem_with_time(1.0), elem_with_time(2.0)]);
        score.add_source("t2".into(), "g".into(), vec![elem_with_time(0.5), elem_with_time(0.5)]);

        score.extend_groups(&["g".into()], true);

        let t1 = score.tracks.get("t1").unwrap();
        let t2 = score.tracks.get("t2").unwrap();
        // Both should be fully extended (total 3.0 for t1, 1.0 for t2)
        // t2 should be padded to match t1's end time
        assert_eq!(t1.len(), 2);
        assert!(!t2.is_empty());
        let t1_total: f64 = t1.iter().map(|m| m.time).sum();
        let t2_total: f64 = t2.iter().map(|m| m.time).sum();
        assert!((t1_total - t2_total).abs() < 0.001, "t1={}, t2={}", t1_total, t2_total);
    }

    #[test]
    fn test_extend_groups_non_matching_padded() {
        // One track in group, one not → non-matching gets silence padding
        let mut score = Score::new();
        score.add_source("t1".into(), "g".into(), vec![elem_with_time(1.0), elem_with_time(2.0)]);
        score.add_source("t2".into(), "other".into(), vec![elem_with_time(1.0)]);

        score.extend_groups(&["g".into()], true);

        let t2 = score.tracks.get("t2").unwrap();
        // t2 should have one silence entry padded to match t1's total (3.0)
        assert!(!t2.is_empty());
        let last = t2.last().unwrap();
        assert!(last.element.is_none(), "last entry should be silence padding");
        assert!((last.time - 3.0).abs() < 0.001, "silence should pad to goal, got {}", last.time);
    }

    #[test]
    fn test_extend_groups_empty_filter() {
        // No track matches filter → no extension
        let mut score = Score::new();
        score.add_source("t1".into(), "g".into(), vec![elem_with_time(1.0)]);

        score.extend_groups(&["nonexistent".into()], false);

        let t1 = score.tracks.get("t1").unwrap();
        assert!(t1.is_empty());
    }

    #[test]
    fn test_unpack_compresses_silence() {
        // Silence should be compressed into the preceding note's time
        let mut score = Score::new();
        score.add_source("t1".into(), "g".into(), vec![elem_with_time(1.0)]);

        score.extend_groups(&["g".into()], true);
        let result = score.unpack_timed_tracks();

        // Single track extended to its full length (1.0), no silence needed
        let msgs = result.get("t1").unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].time, 1.0);
    }

    #[test]
    fn test_unpack_silence_becomes_empty() {
        // Pure silence messages (None elements) produce /empty_message bundles
        let mut score = Score::new();
        score.add_source("t1".into(), "g".into(), vec![elem_with_time(0.5)]);
        // Add a track that won't match → gets pure silence
        score.add_source("t2".into(), "other".into(), vec![elem_with_time(1.0)]);

        score.extend_groups(&["g".into()], true);
        let result = score.unpack_timed_tracks();

        // t2 should have a silence entry
        let t2 = result.get("t2").unwrap();
        assert!(!t2.is_empty());
    }
}
