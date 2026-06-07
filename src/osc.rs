use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::str::FromStr;
use std::time::{Duration, SystemTime};

use bigdecimal::BigDecimal;
use rosc::encoder;
use rosc::{OscMessage, OscPacket, OscTime, OscType};

use jdw_osc_lib::model::TimedOSCPacket;

use crate::full;
use crate::shuttle::ResolvedElement;
use crate::note_utils;

// -- ElementConverter (external ID scheme, frequency resolution, OSC routing) --

/// The type of instrument for OSC routing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InstrumentType {
    Sampler,
    Synth,
    Drone,
}

/// Scale data for resolving shuttle note indices to frequencies.
#[derive(Debug, Clone)]
pub struct ScaleData {
    pub scale_key: String,
    pub scale_type: String,
    pub octave_start: i32,
}

impl Default for ScaleData {
    fn default() -> Self {
        ScaleData {
            scale_key: "c".to_string(),
            scale_type: "maj".to_string(),
            octave_start: 4,
        }
    }
}

/// A resolved element paired with its OSC message.
#[derive(Debug, Clone)]
pub struct ElementMessage {
    pub element: ResolvedElement,
    pub message: OscPacket,
}

/// Stateful converter that resolves shuttle elements to OSC messages with
/// unique external IDs containing the `{nodeId}` template.
#[derive(Debug, Clone)]
pub struct ElementConverter {
    pub instrument_name: String,
    pub common_identifier: String,
    pub instrument_type: InstrumentType,
    pub external_id_override: String,
    pub scale_data: ScaleData,
    pub id_counter: u32,
}

// Default delay in ms for OSC messages sent to the router.
const SC_DELAY_MS: i32 = 70;

/// Check if a resolved element matches a given symbol (e.g., `x` for silence).
fn is_symbol(element: &ResolvedElement, sym: &str) -> bool {
    element.suffix.to_lowercase() == sym
        && element.prefix.is_empty()
        && element.index == 0
}

/// Check if the first `n` characters of `s` match a pattern.
fn begins_with(s: &str, pat: &str) -> bool {
    s.starts_with(pat)
}

/// Strip the first `n` characters from a string.
fn cut_first(s: &str, n: usize) -> String {
    if n >= s.len() {
        String::new()
    } else {
        s[n..].to_string()
    }
}

/// Convert resolved element args to a flat OSC arg list, inserting overrides.
fn args_as_osc(args: &HashMap<String, f64>, overrides: &[OscType]) -> Vec<OscType> {
    let mut osc_args: Vec<OscType> = overrides.to_vec();
    for (key, val) in args {
        if !overrides.iter().any(|o| matches!(o, OscType::String(s) if s == key)) {
            osc_args.push(OscType::String(key.clone()));
            osc_args.push(OscType::Float(*val as f32));
        }
    }
    osc_args
}

impl ElementConverter {
    pub fn new(
        instrument_name: &str,
        common_identifier: &str,
        instrument_type: InstrumentType,
        scale_data: ScaleData,
    ) -> Self {
        ElementConverter {
            instrument_name: instrument_name.to_string(),
            common_identifier: common_identifier.to_string(),
            instrument_type,
            external_id_override: String::new(),
            scale_data,
            id_counter: 0,
        }
    }

    pub fn resolve_message(&mut self, element: &ResolvedElement, transpose_steps: i32) -> Option<ElementMessage> {
        if begins_with(&element.suffix, "@") {
            let override_id = cut_first(&element.suffix, 1);
            Some(ElementMessage {
                element: element.clone(),
                message: self.to_note_mod(element, transpose_steps, &override_id),
            })
        } else if is_symbol(element, "x") {
            Some(ElementMessage {
                element: element.clone(),
                message: OscPacket::Message(OscMessage {
                    addr: "/empty_msg".to_string(),
                    args: vec![],
                }),
            })
        } else if is_symbol(element, ".") {
            None
        } else if is_symbol(element, "§") {
            Some(ElementMessage {
                element: element.clone(),
                message: OscPacket::Message(OscMessage {
                    addr: "/jdw_sc_event_trigger".to_string(),
                    args: vec![
                        OscType::String("loop_started".to_string()),
                        OscType::Int(SC_DELAY_MS),
                    ],
                }),
            })
        } else if begins_with(&element.suffix, "$") {
            let override_id = cut_first(&element.suffix, 1);
            Some(ElementMessage {
                element: element.clone(),
                message: self.to_note_on(element, &override_id, transpose_steps),
            })
        } else if self.instrument_type == InstrumentType::Drone {
            Some(ElementMessage {
                element: element.clone(),
                message: self.to_note_mod(element, transpose_steps, ""),
            })
        } else if self.instrument_type == InstrumentType::Sampler {
            Some(ElementMessage {
                element: element.clone(),
                message: self.to_play_sample(element),
            })
        } else {
            Some(ElementMessage {
                element: element.clone(),
                message: self.to_note_on_timed(element, transpose_steps),
            })
        }
    }

    fn resolve_external_id(&mut self, element: &ResolvedElement) -> String {
        if !element.suffix.is_empty() {
            return element.suffix.clone();
        }
        let node_id = self.id_counter;
        self.id_counter += 1;
        format!(
            "{}_{}_{}{}_{}_{{nodeId}}",
            self.common_identifier,
            self.instrument_name,
            node_id,
            element.index,
            node_id,
        )
    }

    fn resolve_freq(&self, element: &ResolvedElement, transpose_steps: i32) -> f64 {
        if let Some(freq) = element.args.get("freq") {
            return *freq;
        }

        let letter_check = note_utils::note_letter_to_midi(&element.prefix);

        if letter_check == -1 {
            let index = note_utils::resolve_index(
                element.index as i32,
                &self.scale_data.scale_key,
                &self.scale_data.scale_type,
            );
            let octave = self.scale_data.octave_start;
            let extra = if octave > 0 { 12 * (octave + 1) } else { 0 };
            let new_index = (index + extra + transpose_steps) as f64;
            note_utils::midi_to_hz(new_index)
        } else {
            // Letter name with octave number
            let extra = if element.index > 0 { 12 * (element.index as i32 - 1) } else { 0 };
            let new_index = (letter_check + extra + transpose_steps) as f64;
            note_utils::midi_to_hz(new_index)
        }
    }

    fn to_note_mod(&mut self, element: &ResolvedElement, transpose_steps: i32, external_id_override: &str) -> OscPacket {
        let effective_override = if external_id_override.is_empty() {
            self.external_id_override.clone()
        } else {
            external_id_override.to_string()
        };
        let external_id = if effective_override.is_empty() {
            self.resolve_external_id(element)
        } else {
            effective_override
        };
        let freq = self.resolve_freq(element, transpose_steps);
        let osc_args = args_as_osc(&element.args, &[
            OscType::String("freq".to_string()),
            OscType::Float(freq as f32),
        ]);
        OscPacket::Message(OscMessage {
            addr: "/note_modify".to_string(),
            args: {
                let mut v = vec![
                    OscType::String(external_id),
                    OscType::Int(SC_DELAY_MS),
                ];
                v.extend(osc_args);
                v
            },
        })
    }

    fn to_note_on_timed(&mut self, element: &ResolvedElement, transpose_steps: i32) -> OscPacket {
        let freq = self.resolve_freq(element, transpose_steps);
        let external_id = self.resolve_external_id(element);
        let sus = element.args.get("sus").copied().unwrap_or(0.0);
        let gate_time = format!("{}", sus);
        let osc_args = args_as_osc(&element.args, &[
            OscType::String("freq".to_string()),
            OscType::Float(freq as f32),
        ]);
        OscPacket::Message(OscMessage {
            addr: "/note_on_timed".to_string(),
            args: {
                let mut v = vec![
                    OscType::String(self.instrument_name.clone()),
                    OscType::String(external_id),
                    OscType::String(gate_time),
                    OscType::Int(SC_DELAY_MS),
                ];
                v.extend(osc_args);
                v
            },
        })
    }

    fn to_play_sample(&mut self, element: &ResolvedElement) -> OscPacket {
        let freq = self.resolve_freq(element, 0);
        let external_id = self.resolve_external_id(element);
        let osc_args = args_as_osc(&element.args, &[
            OscType::String("freq".to_string()),
            OscType::Float(freq as f32),
        ]);
        OscPacket::Message(OscMessage {
            addr: "/play_sample".to_string(),
            args: {
                let mut v = vec![
                    OscType::String(external_id),
                    OscType::String(self.instrument_name.clone()),
                    OscType::Int(element.index as i32),
                    OscType::String(element.prefix.clone()),
                    OscType::Int(SC_DELAY_MS),
                ];
                v.extend(osc_args);
                v
            },
        })
    }

    fn to_note_on(&mut self, element: &ResolvedElement, external_id_override: &str, transpose_steps: i32) -> OscPacket {
        let external_id = if external_id_override.is_empty() {
            self.resolve_external_id(element)
        } else {
            external_id_override.to_string()
        };
        let freq = self.resolve_freq(element, transpose_steps);
        let osc_args = args_as_osc(&element.args, &[
            OscType::String("freq".to_string()),
            OscType::Float(freq as f32),
        ]);
        OscPacket::Message(OscMessage {
            addr: "/note_on".to_string(),
            args: {
                let mut v = vec![
                    OscType::String(self.instrument_name.clone()),
                    OscType::String(external_id),
                    OscType::Int(SC_DELAY_MS),
                ];
                v.extend(osc_args);
                v
            },
        })
    }
}

fn f64_to_bigdecimal(val: f64) -> BigDecimal {
    BigDecimal::from_str(&format!("{}", val)).unwrap_or_else(|_| BigDecimal::from(1))
}

fn osc_timetag_now() -> OscTime {
    OscTime::try_from(SystemTime::now())
        .unwrap_or_else(|_| OscTime::from((0u32, 0u32)))
}

const ROUTER_DEFAULT: &str = "127.0.0.1:13339";
const DELAY_CONFIGURE_MS: u64 = 5;

/// Configuration for sending OSC messages.
pub struct OscConfig {
    pub router_addr: String,
    pub sequencer_port: i32,
    pub sc_port: i32,
    pub external_id_counter: u64,
}

impl Default for OscConfig {
    fn default() -> Self {
        OscConfig {
            router_addr: ROUTER_DEFAULT.to_string(),
            sequencer_port: 14441,
            sc_port: 13331,
            external_id_counter: 0,
        }
    }
}

/// Send a `/hard_stop` message to the router for the sequencer.
pub fn send_stop(config: &OscConfig) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;
    sock.set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .map_err(|e| format!("set timeout: {}", e))?;

    let msg = OscPacket::Message(OscMessage {
        addr: "/hard_stop".to_string(),
        args: vec![],
    });
    send_to_router(&sock, &config.router_addr, &msg)?;

    let wipe = OscPacket::Message(OscMessage {
        addr: "/wipe_on_finish".to_string(),
        args: vec![],
    });
    send_to_router(&sock, &config.router_addr, &wipe)?;

    Ok(())
}

/// Send a `/free_notes` command matching a track alias.
pub fn send_free_notes(alias: &str, config: &OscConfig) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;
    let msg = OscPacket::Message(OscMessage {
        addr: "/free_notes".to_string(),
        args: vec![OscType::String(alias.to_string())],
    });
    send_to_router(&sock, &config.router_addr, &msg)
}

/// Silence drone synths: send `/note_modify amp=0.0` for each drone section.
pub fn send_silence_drones(billboard: &full::Billboard, config: &OscConfig) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;

    for section in &billboard.sections {
        if !section.header.is_drone {
            continue;
        }
        // Generate a simple external ID pattern that matches drone notes
        let ext_id = format!(".*{}.*", regex::escape(&section.header.instrument));
        let msg = OscPacket::Message(OscMessage {
            addr: "/note_modify".to_string(),
            args: vec![
                OscType::String(ext_id),
                OscType::Int(0),
                OscType::String("amp".to_string()),
                OscType::Float(0.0),
            ],
        });
        send_to_router(&sock, &config.router_addr, &msg)?;
    }
    Ok(())
}

// -- Internal helpers --

fn send_to_router(sock: &UdpSocket, addr: &str, packet: &OscPacket) -> Result<(), String> {
    let target: SocketAddr = addr.parse().map_err(|e| format!("invalid addr: {}", e))?;
    let buf = encoder::encode(packet).map_err(|e| format!("encode: {}", e))?;
    sock.send_to(&buf, target).map_err(|e| format!("send: {}", e))?;
    Ok(())
}

/// Convert a string arg to the most specific OscType.
///
/// Tries int first, then float, falls back to string. This matches
/// Python's `pythonosc` auto-detection in `OscMessageBuilder.add_arg()`.
fn osc_arg_from_str(s: &str) -> OscType {
    if let Ok(i) = s.parse::<i32>() {
        return OscType::Int(i);
    }
    if let Ok(f) = s.parse::<f32>() {
        return OscType::Float(f);
    }
    OscType::String(s.to_string())
}

// ---------------------------------------------------------------------------
// Full Billboard OSC conversion (Stage 5+)
// ---------------------------------------------------------------------------

/// Extract scale data from billboard commands (`/set_scale`), or return a
/// sensible default (C major, octave 4).
fn extract_scale_data(commands: &[full::BillboardCommand]) -> ScaleData {
    for cmd in commands {
        if cmd.address == "/set_scale" && cmd.args.len() >= 3 {
            return ScaleData {
                scale_key: cmd.args[0].clone(),
                scale_type: cmd.args[1].clone(),
                octave_start: cmd.args[2].parse().unwrap_or(4),
            };
        }
    }
    ScaleData::default()
}

/// Send `/create_synthdef` messages for each known SynthDef to the router.
///
/// This replaces the old `/read_scd ~jdw.synth()` approach with the
/// correct OSC protocol (matching the Python `setup()` flow).
pub fn send_full_setup(
    synthdefs: &[crate::synthdefs::SynthDefMessage],
    config: &OscConfig,
) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;

    for def in synthdefs {
        let msg = OscPacket::Message(OscMessage {
            addr: "/create_synthdef".to_string(),
            args: vec![OscType::String(def.content.clone())],
        });
        send_to_router(&sock, &config.router_addr, &msg)?;
        // Delay between messages to prevent dropped packets (matches Python)
        std::thread::sleep(Duration::from_millis(DELAY_CONFIGURE_MS));
    }

    Ok(())
}

/// Build a `timed_msg` bundle for a single timed OSC packet.
///
/// Wire format:
/// ```osc
/// /bundle_info "timed_msg"
/// /timed_msg_info "<relative_beat>"
/// <packet>
/// ```
fn build_timed_msg_bundle(packet: &TimedOSCPacket) -> OscPacket {
    OscPacket::Bundle(rosc::OscBundle {
        timetag: osc_timetag_now(),
        content: vec![
            OscPacket::Message(OscMessage {
                addr: "/bundle_info".to_string(),
                args: vec![OscType::String("timed_msg".to_string())],
            }),
            OscPacket::Message(OscMessage {
                addr: "/timed_msg_info".to_string(),
                args: vec![OscType::String(packet.time.to_string())],
            }),
            packet.packet.clone(),
        ],
    })
}

/// Build an `update_queue` bundle for a single track/alias.
///
/// Wire format:
/// ```osc
/// /bundle_info "update_queue"
/// /update_queue_info "<alias>" <one_shot: 0|1>
/// [bundle containing timed_msg bundles]
/// ```
fn build_update_queue_bundle(
    alias: &str,
    one_shot: bool,
    timed_packets: &[TimedOSCPacket],
) -> OscPacket {
    let timed_bundles: Vec<OscPacket> = timed_packets
        .iter()
        .map(build_timed_msg_bundle)
        .collect();

    OscPacket::Bundle(rosc::OscBundle {
        timetag: osc_timetag_now(),
        content: vec![
            OscPacket::Message(OscMessage {
                addr: "/bundle_info".to_string(),
                args: vec![OscType::String("update_queue".to_string())],
            }),
            OscPacket::Message(OscMessage {
                addr: "/update_queue_info".to_string(),
                args: vec![
                    OscType::String(alias.to_string()),
                    OscType::Int(if one_shot { 1 } else { 0 }),
                ],
            }),
            OscPacket::Bundle(rosc::OscBundle {
                timetag: osc_timetag_now(),
                content: timed_bundles,
            }),
        ],
    })
}

/// Build a `batch_update_queues` bundle for a full billboard update.
///
/// Wire format:
/// ```osc
/// /bundle_info "batch_update_queues"
/// /batch_update_queues_info <stop_missing: 0|1>
/// [bundle containing update_queue bundles]
/// ```
fn build_batch_update_bundle(update_bundles: Vec<OscPacket>, stop_missing: bool) -> OscPacket {
    OscPacket::Bundle(rosc::OscBundle {
        timetag: osc_timetag_now(),
        content: vec![
            OscPacket::Message(OscMessage {
                addr: "/bundle_info".to_string(),
                args: vec![OscType::String("batch_update_queues".to_string())],
            }),
            OscPacket::Message(OscMessage {
                addr: "/batch_update_queues_info".to_string(),
                args: vec![OscType::Int(if stop_missing { 1 } else { 0 })],
            }),
            OscPacket::Bundle(rosc::OscBundle {
                timetag: osc_timetag_now(),
                content: update_bundles,
            }),
        ],
    })
}

/// Determine the sequencer alias for a track in a full Billboard section.
///
/// Uses `group_override` if present, otherwise `{synth}_{section_idx}_{track_idx}`.
fn track_alias(synth: &str, section_index: usize, track: &full::TrackDefinition) -> String {
    track
        .group_override
        .clone()
        .unwrap_or_else(|| format!("{}_{}_{}", synth, section_index, track.index))
}

/// Convert a track's shuttle notation to `TimedOSCPacket`s using an
/// `ElementConverter` for proper OSC routing, external IDs, and freq resolution.
///
/// Each packet's `time` is the note duration (beat offset until the next event).
///
/// Arg precedence (highest to lowest, matching Python):
/// 1. Track overrides (with operators: +, -, *, =)
/// 2. Element inline args (e.g. `c4:amp0.5`)
/// 3. Section args (default + header merged)
fn track_to_timed_packets(
    converter: &mut ElementConverter,
    content: &str,
    default_args: &HashMap<String, f64>,
    header_args: &HashMap<String, f64>,
    track_overrides: &HashMap<String, (char, f64)>,
) -> Result<Vec<TimedOSCPacket>, String> {
    let elements = crate::shuttle::parse(content)?;
    let mut packets = Vec::new();

    for elem in &elements {
        // Start with the element's inline args (these are the base from the shuttle parser)
        let mut merged = elem.clone();

        // Apply section args (default + header) — insert only if not already set by element
        for (k, v) in default_args {
            merged.args.entry(k.clone()).or_insert(*v);
        }
        for (k, v) in header_args {
            merged.args.entry(k.clone()).or_insert(*v);
        }

        // Apply track-level overrides with operators (highest precedence, Python-style)
        for (k, &(op, v)) in track_overrides {
            let current = merged.args.entry(k.clone()).or_insert(0.0);
            match op {
                '*' => *current *= v,
                '+' => *current += v,
                '-' => *current -= v,
                _ => *current = v, // '=' or '_' → replace
            }
        }

        match converter.resolve_message(&merged, 0) {
            None => continue,
            Some(msg) => {
                let note_len = merged.args.get("time").copied().unwrap_or(1.0);
                let time = f64_to_bigdecimal(note_len);
                packets.push(TimedOSCPacket {
                    time,
                    packet: msg.message,
                });
            }
        }
    }

    Ok(packets)
}

/// Maximum UDP datagram payload size (safe limit for Ethernet + OSC overhead).
const MAX_UDP_PACKET: usize = 64000;

/// Determine the group name for a track, matching Python's logic:
/// `track.group_override if set else section.header.group`.
fn track_group_name(track: &full::TrackDefinition, section: &full::SynthSection) -> String {
    track
        .group_override
        .clone()
        .unwrap_or_else(|| section.header.group.clone().unwrap_or_default())
}

/// Send a full Billboard's tracks as sequencer queue updates, using the
/// correct jdw-suite bundle protocol and ElementConverter.
///
/// Respects group filters (`>>>` lines): only tracks whose group is in the
/// last filter are included. If no filters are defined, all tracks are sent.
/// Large bundles are automatically split into multiple UDP packets.
pub fn send_full_queue_update(
    billboard: &full::Billboard,
    config: &OscConfig,
) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;
    let scale_data = extract_scale_data(&billboard.commands);

    // Resolve final group filter (last >>> line); empty = no filter
    let final_filter: Vec<String> = billboard
        .filters
        .last()
        .cloned()
        .unwrap_or_default();
    let has_filter = !final_filter.is_empty();

    // Collect individual track bundles
    let mut update_bundles: Vec<OscPacket> = Vec::new();

    for (si, section) in billboard.sections.iter().enumerate() {
        let header_args = &section.header.default_args;

        for track in &section.tracks {
            if !track.enabled {
                continue;
            }

            // Apply group filter: skip if track's group is not in final filter
            if has_filter {
                let gname = track_group_name(track, section);
                if !final_filter.contains(&gname) {
                    continue;
                }
            }

            let instrument_type = if section.header.is_drone {
                InstrumentType::Drone
            } else if section.header.is_sampler {
                InstrumentType::Sampler
            } else {
                InstrumentType::Synth
            };

            let mut converter = ElementConverter::new(
                &section.header.instrument,
                &track.index.to_string(),
                instrument_type,
                scale_data.clone(),
            );

            if section.header.is_drone {
                converter.external_id_override = format!(
                    "effect_{}_{}",
                    section.header.group.as_deref().unwrap_or(""),
                    track.index
                );
            }

            let timed_packets = track_to_timed_packets(
                &mut converter,
                &track.content,
                &billboard.default_args,
                header_args,
                &track.arg_overrides,
            )?;

            if timed_packets.is_empty() {
                continue;
            }

            let alias = track_alias(&section.header.instrument, si, track);
            update_bundles.push(build_update_queue_bundle(&alias, false, &timed_packets));
        }
    }

    if update_bundles.is_empty() {
        return Ok(());
    }

    // Split into batches that fit within UDP size limits
    let mut start = 0;
    while start < update_bundles.len() {
        let mut end = update_bundles.len();
        loop {
            let is_last = end == update_bundles.len();
            let batch = build_batch_update_bundle(
                update_bundles[start..end].to_vec(),
                is_last,
            );
            let encoded = encoder::encode(&batch).map_err(|e| format!("encode: {}", e))?;
            if encoded.len() < MAX_UDP_PACKET || end == start + 1 {
                send_to_router(&sock, &config.router_addr, &batch)?;
                start = end;
                break;
            }
            end -= 1;
        }
    }

    Ok(())
}

/// Generate a human-readable dump of all OSC packets that `send_full_queue_update`
/// would send, without actually sending anything. Useful for comparing Rust vs Python output.
pub fn dump_queue_update(
    billboard: &full::Billboard,
) -> Vec<String> {
    let scale_data = extract_scale_data(&billboard.commands);
    let final_filter: Vec<String> = billboard
        .filters
        .last()
        .cloned()
        .unwrap_or_default();
    let has_filter = !final_filter.is_empty();
    let mut lines = Vec::new();

    for (si, section) in billboard.sections.iter().enumerate() {
        let header_args = &section.header.default_args;

        for track in &section.tracks {
            if !track.enabled { continue; }
            if has_filter {
                let gname = track_group_name(track, section);
                if !final_filter.contains(&gname) { continue; }
            }

            let instrument_type = if section.header.is_drone { InstrumentType::Drone }
                else if section.header.is_sampler { InstrumentType::Sampler }
                else { InstrumentType::Synth };

            let mut converter = ElementConverter::new(
                &section.header.instrument, &track.index.to_string(),
                instrument_type, scale_data.clone(),
            );

            if section.header.is_drone {
                converter.external_id_override = format!("effect_{}_{}",
                    section.header.group.as_deref().unwrap_or(""), track.index);
            }

            let alias = track_alias(&section.header.instrument, si, track);
            lines.push(format!("--- Track: {} (‘{}’ section {}) ---", alias, section.header.instrument, si));

            match track_to_timed_packets(&mut converter, &track.content,
                &billboard.default_args, header_args, &track.arg_overrides)
            {
                Err(e) => lines.push(format!("  ERROR: {}", e)),
                Ok(packets) => {
                    for (i, tp) in packets.iter().enumerate() {
                        match &tp.packet {
                            OscPacket::Message(msg) => {
                                let args: Vec<String> = msg.args.iter().map(|a| osc_type_to_string(a)).collect();
                                lines.push(format!("  [{}] {} (t={}) {}",
                                    i, msg.addr, tp.time, args.join(" ")));
                            }
                            OscPacket::Bundle(_) => {
                                lines.push(format!("  [{}] <bundle> (t={})", i, tp.time));
                            }
                        }
                    }
                }
            }
        }
    }

    lines
}

fn osc_type_to_string(t: &OscType) -> String {
    match t {
        OscType::String(s) => format!("\"{}\"", s),
        OscType::Int(i) => format!("{}", i),
        OscType::Float(f) => format!("{}", f),
        _ => format!("{:?}", t),
    }
}

/// Send all commands from a full Billboard to the OSC router.
pub fn send_full_commands(
    billboard: &full::Billboard,
    config: &OscConfig,
) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;

    for cmd in &billboard.commands {
        let args: Vec<OscType> = cmd.args.iter().map(|a| osc_arg_from_str(a)).collect();
        let msg = OscPacket::Message(OscMessage {
            addr: cmd.address.clone(),
            args,
        });
        send_to_router(&sock, &config.router_addr, &msg)?;
        std::thread::sleep(Duration::from_millis(DELAY_CONFIGURE_MS));
    }

    Ok(())
}

/// Full pipeline: setup + queue update + commands for a full Billboard.
pub fn send_full_billboard(
    billboard: &full::Billboard,
    synthdefs: &[crate::synthdefs::SynthDefMessage],
    config: &OscConfig,
) -> Result<(), String> {
    send_full_setup(synthdefs, config)?;
    send_full_queue_update(billboard, config)?;
    send_full_commands(billboard, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::full;
    use std::collections::HashMap;

    #[test]
    fn test_track_to_timed_packets_basic() {
        let mut converter = ElementConverter::new("testSynth", "0", InstrumentType::Synth, ScaleData::default());
        let empty: HashMap<String, f64> = HashMap::new();
        let empty_overrides: HashMap<String, (char, f64)> = HashMap::new();
        let packets = track_to_timed_packets(&mut converter, "c4 d4 e4", &empty, &empty, &empty_overrides).unwrap();
        assert_eq!(packets.len(), 3);

        assert_eq!(packets[0].time, f64_to_bigdecimal(1.0));
        assert_eq!(packets[1].time, f64_to_bigdecimal(1.0));
        assert_eq!(packets[2].time, f64_to_bigdecimal(1.0));

        if let OscPacket::Message(ref msg) = packets[0].packet {
            assert_eq!(msg.addr, "/note_on_timed");
            assert_eq!(msg.args[0], OscType::String("testSynth".to_string()));
        } else {
            panic!("Expected message");
        }
    }

    #[test]
    fn test_track_to_timed_packets_with_args() {
        let mut converter = ElementConverter::new("s", "0", InstrumentType::Synth, ScaleData::default());
        let empty: HashMap<String, f64> = HashMap::new();
        let empty_overrides: HashMap<String, (char, f64)> = HashMap::new();
        let packets = track_to_timed_packets(&mut converter, "c4:time0.5 d4:time1.5 e4", &empty, &empty, &empty_overrides).unwrap();

        assert_eq!(packets.len(), 3);
        assert_eq!(packets[0].time, f64_to_bigdecimal(0.5));
        assert_eq!(packets[1].time, f64_to_bigdecimal(1.5));
        assert_eq!(packets[2].time, f64_to_bigdecimal(1.0));
    }

    #[test]
    fn test_track_to_timed_packets_sampler() {
        let mut converter = ElementConverter::new("Roland808", "0", InstrumentType::Sampler, ScaleData::default());
        let empty: HashMap<String, f64> = HashMap::new();
        let empty_overrides: HashMap<String, (char, f64)> = HashMap::new();
        let packets = track_to_timed_packets(&mut converter, "14 26 32", &empty, &empty, &empty_overrides).unwrap();
        assert_eq!(packets.len(), 3);
        if let OscPacket::Message(ref msg) = packets[0].packet {
            assert_eq!(msg.addr, "/play_sample");
            assert_eq!(msg.args[1], OscType::String("Roland808".to_string()));
        } else {
            panic!("Expected message");
        }
    }

    #[test]
    fn test_build_timed_msg_bundle_structure() {
        let packet = TimedOSCPacket {
            time: f64_to_bigdecimal(0.5),
            packet: OscPacket::Message(OscMessage {
                addr: "/note_on_timed".to_string(),
                args: vec![],
            }),
        };
        let bundle = build_timed_msg_bundle(&packet);

        match bundle {
            OscPacket::Bundle(b) => {
                assert_eq!(b.content.len(), 3);
                // First message: /bundle_info "timed_msg"
                if let OscPacket::Message(ref m) = b.content[0] {
                    assert_eq!(m.addr, "/bundle_info");
                    assert_eq!(m.args[0], OscType::String("timed_msg".to_string()));
                } else {
                    panic!("Expected message at index 0");
                }
                // Second message: /timed_msg_info "0.5"
                if let OscPacket::Message(ref m) = b.content[1] {
                    assert_eq!(m.addr, "/timed_msg_info");
                    assert_eq!(m.args[0], OscType::String("0.5".to_string()));
                } else {
                    panic!("Expected message at index 1");
                }
            }
            _ => panic!("Expected bundle"),
        }
    }

    #[test]
    fn test_build_update_queue_bundle_structure() {
        let packets = vec![
            TimedOSCPacket {
                time: f64_to_bigdecimal(1.0),
                packet: OscPacket::Message(OscMessage {
                    addr: "/note_on_timed".to_string(),
                    args: vec![],
                }),
            },
        ];
        let bundle = build_update_queue_bundle("testAlias", true, &packets);

        match bundle {
            OscPacket::Bundle(b) => {
                assert_eq!(b.content.len(), 3);
                // /bundle_info "update_queue"
                if let OscPacket::Message(ref m) = b.content[0] {
                    assert_eq!(m.addr, "/bundle_info");
                    assert_eq!(m.args[0], OscType::String("update_queue".to_string()));
                } else {
                    panic!("Expected message at 0");
                }
                // /update_queue_info "testAlias" 1
                if let OscPacket::Message(ref m) = b.content[1] {
                    assert_eq!(m.addr, "/update_queue_info");
                    assert_eq!(m.args[0], OscType::String("testAlias".to_string()));
                    assert_eq!(m.args[1], OscType::Int(1));
                } else {
                    panic!("Expected message at 1");
                }
                // Inner bundle containing timed_msg bundles
                if let OscPacket::Bundle(ref inner) = b.content[2] {
                    assert_eq!(inner.content.len(), 1);
                } else {
                    panic!("Expected bundle at 2");
                }
            }
            _ => panic!("Expected bundle"),
        }
    }

    #[test]
    fn test_build_batch_update_bundle_structure() {
        let inner = OscPacket::Bundle(rosc::OscBundle {
            timetag: osc_timetag_now(),
            content: vec![],
        });
        let batch = build_batch_update_bundle(vec![inner], true);

        match batch {
            OscPacket::Bundle(b) => {
                assert_eq!(b.content.len(), 3);
                if let OscPacket::Message(ref m) = b.content[0] {
                    assert_eq!(m.addr, "/bundle_info");
                    assert_eq!(
                        m.args[0],
                        OscType::String("batch_update_queues".to_string())
                    );
                } else {
                    panic!("Expected message at 0");
                }
                if let OscPacket::Message(ref m) = b.content[1] {
                    assert_eq!(m.addr, "/batch_update_queues_info");
                    assert_eq!(m.args[0], OscType::Int(1));
                } else {
                    panic!("Expected message at 1");
                }
            }
            _ => panic!("Expected bundle"),
        }
    }

    #[test]
    fn test_track_alias_with_override() {
        let track = full::TrackDefinition {
            content: "c4".to_string(),
            group_override: Some("melody".to_string()),
            arg_overrides: HashMap::new(),
            index: 2,
            enabled: true,
        };
        assert_eq!(track_alias("synth", 1, &track), "melody");
    }

    #[test]
    fn test_track_alias_without_override() {
        let track = full::TrackDefinition {
            content: "c4".to_string(),
            group_override: None,
            arg_overrides: HashMap::new(),
            index: 2,
            enabled: true,
        };
        assert_eq!(track_alias("synth", 1, &track), "synth_1_2");
    }

    #[test]
    fn test_parse_real_bbd_gong() {
        let source = include_str!("../../jdw-pycompose/songs/gong.bbd");
        let bb = full::parse(source);

        // 10 commands + 4 UPDATE_COMMANDs... actually let's count
        // Lines 1-19: 19 UPDATE_COMMAND, Line 21: COMMAND, etc.
        assert!(!bb.commands.is_empty(), "gong.bbd should have commands");
        assert!(!bb.sections.is_empty(), "gong.bbd should have sections");

        // Check default args
        assert!(bb.default_args.contains_key("time"));
        assert!(bb.default_args.contains_key("amp"));
    }

    #[test]
    fn test_parse_real_bbd_arena() {
        let source = include_str!("../../jdw-pycompose/songs/arena.bbd");
        let bb = full::parse(source);

        assert!(!bb.commands.is_empty());
        assert!(!bb.sections.is_empty());
        assert!(bb.default_args.contains_key("amp"));

        // Check that some tracks have metadata
        let has_meta = bb.sections.iter().any(|s| {
            s.tracks
                .iter()
                .any(|t| t.group_override.is_some())
        });
        assert!(has_meta, "arena.bbd should have tracks with group overrides");
    }

    #[test]
    fn test_parse_real_bbd_rattlesnake() {
        let source = include_str!("../../jdw-pycompose/songs/rattlesnake.bbd");
        let bb = full::parse(source);

        assert!(!bb.commands.is_empty());
        assert!(!bb.sections.is_empty());
        // Filters are collected regardless of position (matching Python)
        // rattlesnake.bbd has >>> lines after commands
        assert!(!bb.sections[0].tracks.is_empty());
        // Check that tracks with metadata strip content correctly
        let has_meta = bb.sections.iter().any(|s| {
            s.tracks
                .iter()
                .any(|t| t.group_override.is_some())
        });
        assert!(has_meta);
    }
}


