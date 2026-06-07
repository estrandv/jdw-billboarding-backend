use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::str::FromStr;
use std::time::SystemTime;

use bigdecimal::BigDecimal;
use rosc::encoder;
use rosc::{OscMessage, OscPacket, OscTime, OscType};

use jdw_osc_lib::model::TimedOSCPacket;

use crate::full::{self, resolve_args};

fn f64_to_bigdecimal(val: f64) -> BigDecimal {
    BigDecimal::from_str(&format!("{}", val)).unwrap_or_else(|_| BigDecimal::from(1))
}

fn osc_timetag_now() -> OscTime {
    OscTime::try_from(SystemTime::now())
        .unwrap_or_else(|_| OscTime::from((0u32, 0u32)))
}

const ROUTER_DEFAULT: &str = "127.0.0.1:13339";

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

// -- Internal helpers --

fn send_to_router(sock: &UdpSocket, addr: &str, packet: &OscPacket) -> Result<(), String> {
    let target: SocketAddr = addr.parse().map_err(|e| format!("invalid addr: {}", e))?;
    let buf = encoder::encode(packet).map_err(|e| format!("encode: {}", e))?;
    sock.send_to(&buf, target).map_err(|e| format!("send: {}", e))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Full Billboard OSC conversion (Stage 5+)
// ---------------------------------------------------------------------------

/// Collect all unique synths from a full Billboard and send `/read_scd` setup.
pub fn send_full_setup(billboard: &full::Billboard, config: &OscConfig) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;

    let mut synths_seen = std::collections::HashSet::new();
    for section in &billboard.sections {
        let synth = &section.header.instrument;
        if synths_seen.insert(synth.clone()) {
            let msg = OscPacket::Message(OscMessage {
                addr: "/read_scd".to_string(),
                args: vec![OscType::String(format!(
                    "~jdw.synth(\"{}\").add;",
                    synth
                ))],
            });
            send_to_router(&sock, &config.router_addr, &msg)?;
        }
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

/// Convert a track's shuttle notation to `TimedOSCPacket`s with relative beats.
///
/// Each packet's `time` is the note duration (beat offset until the next event),
/// matching what the sequencer's `to_sequence()` expects.
pub fn full_track_to_timed_packets(
    synth_name: &str,
    section_index: usize,
    track: &full::TrackDefinition,
    resolved_args: &HashMap<String, f64>,
    config: &OscConfig,
) -> Result<Vec<TimedOSCPacket>, String> {
    let elements = crate::shuttle::parse(&track.content)?;
    let mut packets = Vec::new();

    for elem in &elements {
        let mut full_args = resolved_args.clone();
        for (k, v) in &elem.args {
            full_args.insert(k.clone(), *v);
        }

        let note_len = full_args.get("time").copied().unwrap_or(1.0);
        let gate = full_args.get("sus").copied().unwrap_or(note_len * 0.8);
        let pitch = format!("{}{}{}", elem.prefix, elem.index, elem.suffix);

        let msg = OscMessage {
            addr: "/note_on_timed".to_string(),
            args: vec![
                OscType::String(synth_name.to_string()),
                OscType::String(format!(
                    "{}_{}_{}_{}_{{nodeId}}",
                    synth_name, section_index, track.index, config.external_id_counter
                )),
                OscType::Float(gate as f32),
                OscType::Float(0.0),
                OscType::String(pitch),
            ],
        };

        let time = f64_to_bigdecimal(note_len);
        packets.push(TimedOSCPacket {
            time,
            packet: OscPacket::Message(msg),
        });
    }

    Ok(packets)
}

/// Send a full Billboard's tracks as sequencer queue updates, using the
/// correct jdw-suite bundle protocol.
pub fn send_full_queue_update(
    billboard: &full::Billboard,
    config: &OscConfig,
) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;
    let mut update_bundles = Vec::new();

    for (si, section) in billboard.sections.iter().enumerate() {
        let header_args = &section.header.default_args;

        for track in &section.tracks {
            if !track.enabled {
                continue;
            }
            let resolved = resolve_args(&billboard.default_args, header_args, &track.arg_overrides);
            let timed_packets = full_track_to_timed_packets(
                &section.header.instrument,
                si,
                track,
                &resolved,
                config,
            )?;

            if timed_packets.is_empty() {
                continue;
            }

            let alias = track_alias(&section.header.instrument, si, track);
            // one_shot = false for looped tracks (they keep repeating)
            let bundle = build_update_queue_bundle(&alias, false, &timed_packets);
            update_bundles.push(bundle);
        }
    }

    if !update_bundles.is_empty() {
        let batch = build_batch_update_bundle(update_bundles, true);
        send_to_router(&sock, &config.router_addr, &batch)?;
    }

    Ok(())
}

/// Send all commands from a full Billboard to the OSC router.
pub fn send_full_commands(
    billboard: &full::Billboard,
    config: &OscConfig,
) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;

    for cmd in &billboard.commands {
        let args: Vec<OscType> = cmd.args.iter().map(|a| OscType::String(a.clone())).collect();
        let msg = OscPacket::Message(OscMessage {
            addr: cmd.address.clone(),
            args,
        });
        send_to_router(&sock, &config.router_addr, &msg)?;
    }

    Ok(())
}

/// Full pipeline: setup + queue update + commands for a full Billboard.
pub fn send_full_billboard(
    billboard: &full::Billboard,
    config: &OscConfig,
) -> Result<(), String> {
    send_full_setup(billboard, config)?;
    send_full_queue_update(billboard, config)?;
    send_full_commands(billboard, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::full;
    use std::collections::HashMap;

    #[test]
    fn test_full_track_to_timed_packets_basic() {
        let track = full::TrackDefinition {
            content: "c4 d4 e4".to_string(),
            group_override: None,
            arg_overrides: HashMap::new(),
            index: 0,
            enabled: true,
        };
        let config = OscConfig::default();
        let resolved = HashMap::new();
        let packets =
            full_track_to_timed_packets("testSynth", 0, &track, &resolved, &config).unwrap();
        assert_eq!(packets.len(), 3);

        // Each note has note_len=1.0 (default), so each packet.time = 1.0
        assert_eq!(packets[0].time, f64_to_bigdecimal(1.0));
        assert_eq!(packets[1].time, f64_to_bigdecimal(1.0));
        assert_eq!(packets[2].time, f64_to_bigdecimal(1.0));

        if let OscPacket::Message(ref msg) = packets[0].packet {
            assert_eq!(msg.addr, "/note_on_timed");
            assert_eq!(msg.args[0], OscType::String("testSynth".to_string()));
            assert_eq!(msg.args[4], OscType::String("c4".to_string()));
        } else {
            panic!("Expected message");
        }
    }

    #[test]
    fn test_full_track_to_timed_packets_with_args() {
        let track = full::TrackDefinition {
            content: "c4:time0.5 d4:time1.5 e4".to_string(),
            group_override: None,
            arg_overrides: HashMap::new(),
            index: 0,
            enabled: true,
        };
        let config = OscConfig::default();
        let resolved = HashMap::new();
        let packets =
            full_track_to_timed_packets("s", 0, &track, &resolved, &config).unwrap();

        assert_eq!(packets.len(), 3);
        // c4: time=0.5, d4: time=1.5, e4: time=1.0 (default)
        assert_eq!(packets[0].time, f64_to_bigdecimal(0.5));
        assert_eq!(packets[1].time, f64_to_bigdecimal(1.5));
        assert_eq!(packets[2].time, f64_to_bigdecimal(1.0));
    }

    #[test]
    fn test_full_track_to_timed_packets_drum_samples() {
        let track = full::TrackDefinition {
            content: "14 26 32".to_string(),
            group_override: None,
            arg_overrides: HashMap::new(),
            index: 0,
            enabled: true,
        };
        let config = OscConfig::default();
        let packets =
            full_track_to_timed_packets("Roland808", 0, &track, &HashMap::new(), &config).unwrap();
        assert_eq!(packets.len(), 3);
        if let OscPacket::Message(ref msg) = packets[0].packet {
            assert_eq!(msg.args[0], OscType::String("Roland808".to_string()));
            assert_eq!(msg.args[4], OscType::String("14".to_string()));
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
        // Filters appear after commands break the chain, so they're orphans
        assert!(bb.filters.is_empty());
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
