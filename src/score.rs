/// Score — timeline composition for NRT recording.
///
/// Port of jdw-pycompose's `nrt_scoring.py` Score class.
/// Uses BigDecimal for exact beat arithmetic (matching Python's Decimal).
use bigdecimal::BigDecimal;
use std::collections::HashMap;

/// A source track: sequence of beat durations.
#[derive(Debug, Clone)]
struct TrackSource {
    durations: Vec<BigDecimal>,
    group_name: String,
}

/// A single entry in the score timeline.
#[derive(Debug, Clone)]
pub struct ScoreMessage {
    /// Index into the source track's elements, or `None` for silence padding.
    /// This maps back to the original element later during OSC conversion.
    pub source_index: Option<usize>,
    /// Beat duration of this entry.
    pub time: BigDecimal,
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
        durations: Vec<BigDecimal>,
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

    /// Return the maximum total beat time across all tracks (matching Python's Score.get_end_time).
    pub fn get_end_time(&self) -> BigDecimal {
        self.tracks
            .values()
            .map(|msgs| msgs.iter().map(|m| m.time.clone()).sum())
            .max()
            .unwrap_or_else(|| BigDecimal::from(0))
    }

    /// Debug: return sum of source durations for each track.
    pub fn source_sums(&self) -> HashMap<String, BigDecimal> {
        self.track_sources
            .iter()
            .map(|(name, src)| (name.clone(), src.durations.iter().cloned().sum()))
            .collect()
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
            .max_by(|a, b| {
                let a_total: BigDecimal = self.track_sources[a.as_str()].durations.iter().cloned().sum();
                let b_total: BigDecimal = self.track_sources[b.as_str()].durations.iter().cloned().sum();
                a_total.partial_cmp(&b_total).unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned()
            .unwrap();

        {
            let src = &self.track_sources[&longest_name];
            let track = self.tracks.get_mut(&longest_name).unwrap();
            for idx in 0..src.durations.len() {
                track.push(ScoreMessage {
                    source_index: Some(idx),
                    time: src.durations[idx].clone(),
                });
            }
        }

        let goal_time: BigDecimal = self.tracks[&longest_name].iter().map(|m| m.time.clone()).sum();

        // Python's extend_groups iterates ALL tracks (not just matching) and pads
        // non-matching tracks to goal_time via silence (never extends with elements).
        // Matching tracks get extended if there's room for a full repeat, else padded.
        for (name, src) in &self.track_sources {
            if *name == longest_name { continue; }
            let is_matching = matching.contains(name);
            let src_duration: BigDecimal = src.durations.iter().cloned().sum();
            let track = self.tracks.get_mut(name).unwrap();

            let current_time: BigDecimal = track.iter().map(|m| m.time.clone()).sum();
            if current_time >= goal_time { continue; }
            let mut remaining = goal_time.clone() - &current_time;

            while is_matching && remaining >= src_duration && src_duration > BigDecimal::from(0) {
                for idx in 0..src.durations.len() {
                    track.push(ScoreMessage {
                        source_index: Some(idx),
                        time: src.durations[idx].clone(),
                    });
                }
                let new_time: BigDecimal = track.iter().map(|m| m.time.clone()).sum();
                remaining = goal_time.clone() - new_time;
            }

            if remaining > BigDecimal::from(0) {
                track.push(ScoreMessage { source_index: None, time: remaining });
            }
        }
    }

    /// Convert score tracks to timed duration lists.
    /// Returns (time, source_index_or_none) for each entry.
    pub fn unpack_timed_entries(&self) -> HashMap<String, Vec<(BigDecimal, Option<usize>)>> {
        let mut result = HashMap::new();

        for (name, messages) in &self.tracks {
            let mut timed: Vec<(BigDecimal, Option<usize>)> = Vec::new();
            let mut pending = BigDecimal::from(0);

            for msg in messages {
                match msg.source_index {
                    Some(_) => {
                        if pending > BigDecimal::from(0) {
                            timed.push((pending.clone(), None));
                            pending = BigDecimal::from(0);
                        }
                        timed.push((msg.time.clone(), msg.source_index));
                    }
                    None => { pending += msg.time.clone(); }
                }
            }

            if pending > BigDecimal::from(0) && timed.is_empty() {
                timed.push((pending, None));
            } else if pending > BigDecimal::from(0) {
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
    use std::str::FromStr;

    fn bd(s: &str) -> BigDecimal {
        BigDecimal::from_str(s).unwrap()
    }

    #[test]
    fn test_score_add_source() {
        let mut score = Score::new();
        score.add_source("t1".into(), "gA".into(), vec![bd("0.5"), bd("1.0")]);
        assert_eq!(score.track_sources["t1"].durations.len(), 2);
    }

    #[test]
    fn test_extend_groups_same_length() {
        let mut score = Score::new();
        score.add_source("t1".into(), "g".into(), vec![bd("1.0"), bd("2.0")]);
        score.add_source("t2".into(), "g".into(), vec![bd("0.5"), bd("0.5")]);
        score.extend_groups(&["g".into()], true);
        let t1 = &score.tracks["t1"];
        let t2 = &score.tracks["t2"];
        let t1_t: BigDecimal = t1.iter().map(|m| m.time.clone()).sum();
        let t2_t: BigDecimal = t2.iter().map(|m| m.time.clone()).sum();
        assert_eq!(t1_t, t2_t);
    }

    #[test]
    fn test_non_matching_track_padded_to_goal() {
        // Python pads non-matching tracks to goal_time, not leaves them empty.
        let mut score = Score::new();
        score.add_source("t1".into(), "g".into(), vec![bd("1.0"), bd("2.0")]);
        score.add_source("t2".into(), "other".into(), vec![bd("1.0")]);
        score.extend_groups(&["g".into()], true);
        let t2 = &score.tracks["t2"];
        let t2_total: BigDecimal = t2.iter().map(|m| m.time.clone()).sum();
        let expected: BigDecimal = score.tracks["t1"].iter().map(|m| m.time.clone()).sum();
        assert_eq!(t2_total, expected, "t2={} expected={}", t2_total, expected);
        // t2 should be silence (source_index is None)
        assert!(t2.iter().all(|m| m.source_index.is_none()));
    }
}
