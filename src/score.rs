/// Score — timeline composition for NRT recording.
///
/// Port of jdw-pycompose's `nrt_scoring.py` Score class.
/// Works with durations only (f64) for timeline math. The actual OSC
/// conversion happens outside the Score, using ElementConverter.
use std::collections::HashMap;

/// A source track: sequence of beat durations.
#[derive(Debug, Clone)]
struct TrackSource {
    durations: Vec<f64>,
    group_name: String,
}

/// A single entry in the score timeline.
#[derive(Debug, Clone)]
pub struct ScoreMessage {
    /// Index into the source track's elements, or `None` for silence padding.
    /// This maps back to the original element later during OSC conversion.
    pub source_index: Option<usize>,
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

    /// Register a track source with its element durations.
    pub fn add_source(
        &mut self,
        track_name: String,
        group_name: String,
        durations: Vec<f64>,
    ) {
        self.track_sources.insert(
            track_name.clone(),
            TrackSource {
                durations,
                group_name,
            },
        );
        self.tracks.insert(track_name, Vec::new());
    }

    /// Extend tracks matching any of `group_names` into the score timeline.
    pub fn extend_groups(&mut self, group_names: &[String], also_extend_groupless: bool) {
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

        let longest_name = matching
            .iter()
            .max_by_key(|name| {
                let total: f64 = self.track_sources[*name].durations.iter().sum();
                (total * 1000.0) as i64
            })
            .cloned()
            .unwrap();

        {
            let src = &self.track_sources[&longest_name];
            let track = self.tracks.get_mut(&longest_name).unwrap();
            for idx in 0..src.durations.len() {
                track.push(ScoreMessage {
                    source_index: Some(idx),
                    time: src.durations[idx],
                });
            }
        }

        let goal_time: f64 = self.tracks[&longest_name].iter().map(|m| m.time).sum();

        for name in &matching {
            if *name == longest_name { continue; }
            let src = self.track_sources[name].clone();
            let src_duration: f64 = src.durations.iter().sum();
            let track = self.tracks.get_mut(name).unwrap();

            let mut current_time: f64 = track.iter().map(|m| m.time).sum();
            let mut remaining = goal_time - current_time;

            while remaining >= src_duration && src_duration > 0.0 {
                for idx in 0..src.durations.len() {
                    track.push(ScoreMessage {
                        source_index: Some(idx),
                        time: src.durations[idx],
                    });
                }
                current_time += src_duration;
                remaining = goal_time - current_time;
            }

            if remaining > 0.0 {
                track.push(ScoreMessage { source_index: None, time: remaining });
            }
        }

        for name in self.track_sources.keys() {
            if matching.contains(name) { continue; }
            let track = self.tracks.get_mut(name).unwrap();
            if track.is_empty() {
                track.push(ScoreMessage { source_index: None, time: goal_time });
            }
        }
    }

    /// Convert score tracks to timed duration lists.
    /// Returns (time, source_index_or_none) for each entry.
    pub fn unpack_timed_entries(&self) -> HashMap<String, Vec<(f64, Option<usize>)>> {
        let mut result = HashMap::new();

        for (name, messages) in &self.tracks {
            let mut timed: Vec<(f64, Option<usize>)> = Vec::new();
            let mut pending: f64 = 0.0;

            for msg in messages {
                match msg.source_index {
                    Some(_) => {
                        if pending > 0.0 {
                            if let Some(last) = timed.last_mut() { last.0 += pending; }
                            pending = 0.0;
                        }
                        timed.push((msg.time, msg.source_index));
                    }
                    None => { pending += msg.time; }
                }
            }

            if pending > 0.0 && timed.is_empty() {
                timed.push((pending, None));
            } else if pending > 0.0 {
                if let Some(last) = timed.last_mut() { last.0 += pending; }
            }

            result.insert(name.clone(), timed);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_add_source() {
        let mut score = Score::new();
        score.add_source("t1".into(), "gA".into(), vec![0.5, 1.0]);
        assert_eq!(score.track_sources["t1"].durations.len(), 2);
    }

    #[test]
    fn test_extend_groups_same_length() {
        let mut score = Score::new();
        score.add_source("t1".into(), "g".into(), vec![1.0, 2.0]);
        score.add_source("t2".into(), "g".into(), vec![0.5, 0.5]);
        score.extend_groups(&["g".into()], true);
        let t1 = &score.tracks["t1"];
        let t2 = &score.tracks["t2"];
        let t1_t: f64 = t1.iter().map(|m| m.time).sum();
        let t2_t: f64 = t2.iter().map(|m| m.time).sum();
        assert!((t1_t - t2_t).abs() < 0.001);
    }

    #[test]
    fn test_extend_padding() {
        let mut score = Score::new();
        score.add_source("t1".into(), "g".into(), vec![1.0, 2.0]);
        score.add_source("t2".into(), "other".into(), vec![1.0]);
        score.extend_groups(&["g".into()], true);
        let t2 = &score.tracks["t2"];
        assert!(t2.last().unwrap().source_index.is_none());
    }
}
