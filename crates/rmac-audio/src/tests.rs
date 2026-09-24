//! Focused audio service contracts.

use super::notification::*;
use super::*;
use crate::fake::SystemAudioService;

/// Runs the read-only half of the [`crate::fake::AudioService`] contract
/// against the real PipeWire backend. `#[ignore]`d because it needs a
/// running audio graph; run it deliberately on the reference laptop with
/// `cargo test -p rmac-audio -- --ignored`.
#[test]
#[ignore = "needs a live PipeWire session; run on the reference laptop"]
fn system_audio_service_snapshot_is_well_formed() {
    contract::assert_audio_service_is_observable(&SystemAudioService);
}

#[test]
fn original_notification_chime_is_bounded_well_formed_pcm() {
    let wav = default_notification_wav();
    assert!(wav.len() < MAX_NOTIFICATION_SOUND_BYTES);
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(&wav[12..16], b"fmt ");
    assert_eq!(&wav[36..40], b"data");
    assert_eq!(
        u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize,
        wav.len() - 8
    );
    assert_eq!(
        u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize,
        wav.len() - 44
    );
    assert!(wav[44..].chunks_exact(2).any(|sample| sample != [0, 0]));
    validate_notification_sound(NotificationSoundFormat::WavPcm, wav).unwrap();
}

#[test]
fn encoded_notification_sound_boundary_checks_format_size_and_debug() {
    let opus = [b"OggS".as_slice(), &[0; 24], b"OpusHead"].concat();
    let vorbis = [b"OggS".as_slice(), &[0; 24], b"\x01vorbis"].concat();
    validate_notification_sound(NotificationSoundFormat::OggOpus, &opus).unwrap();
    validate_notification_sound(NotificationSoundFormat::OggVorbis, &vorbis).unwrap();
    assert_eq!(
        validate_notification_sound(NotificationSoundFormat::OggVorbis, &opus),
        Err(NotificationPlaybackError::new(
            NotificationPlaybackErrorKind::InvalidSound
        ))
    );
    assert_eq!(
        validate_notification_sound(
            NotificationSoundFormat::WavPcm,
            &vec![0; MAX_NOTIFICATION_SOUND_BYTES + 1],
        ),
        Err(NotificationPlaybackError::new(
            NotificationPlaybackErrorKind::InvalidSound
        ))
    );
    let debug = format!(
        "{:?}",
        NotificationSound::Encoded {
            format: NotificationSoundFormat::OggOpus,
            bytes: &opus,
        }
    );
    assert!(!debug.contains("OpusHead"));
    assert!(debug.contains(&opus.len().to_string()));
}

#[test]
fn node_level_matches_wpctl_cubic_volume_scale() {
    // Real `pw-dump` channelVolumes are linear; wpctl/pavucontrol display the
    // cube root. 8e-6 is the exact real-hardware value the reference laptop
    // reported as `wpctl`-displayed 0.02 (2%); 0.343 -> 0.7 (70%) verifies a
    // non-boundary value on the same curve.
    let quiet = serde_json::json!({
        "info": {"params": {"Props": [
            {"channelVolumes": [8e-6, 8e-6], "mute": false}
        ]}}
    });
    assert_eq!(
        parse_node_level(&quiet),
        Some(Level {
            volume: 2,
            muted: false
        })
    );

    let full_muted = serde_json::json!({
        "info": {"params": {"Props": [
            {"channelVolumes": [1.0, 1.0], "mute": true}
        ]}}
    });
    assert_eq!(
        parse_node_level(&full_muted),
        Some(Level {
            volume: 100,
            muted: true
        })
    );

    let mid = serde_json::json!({
        "info": {"params": {"Props": [{"channelVolumes": [0.343], "mute": false}]}}
    });
    assert_eq!(parse_node_level(&mid).unwrap().volume, 70);
}

#[test]
fn node_level_finds_the_volume_props_entry_among_alsa_route_entries() {
    // Real ALSA-backed sinks/sources have a *second* Props entry (`device`,
    // `deviceName`, `cardName`, …) with no `channelVolumes` field at all;
    // the parser must not assume the Props array has exactly one entry.
    let realistic = serde_json::json!({
        "info": {"params": {"Props": [
            {"volume": 1.0, "mute": false, "channelVolumes": [1.0, 1.0], "channelMap": ["FL", "FR"]},
            {"device": "front:1", "deviceName": "", "cardName": ""}
        ]}}
    });
    assert_eq!(
        parse_node_level(&realistic),
        Some(Level {
            volume: 100,
            muted: false
        })
    );

    let missing_mute =
        serde_json::json!({"info": {"params": {"Props": [{"channelVolumes": [1.0]}]}}});
    assert!(parse_node_level(&missing_mute).is_none());

    let ambiguous = serde_json::json!({"info": {"params": {"Props": [
        {"channelVolumes": [1.0], "mute": false},
        {"channelVolumes": [0.5], "mute": false}
    ]}}});
    assert!(parse_node_level(&ambiguous).is_none());
}

#[test]
fn graph_devices_marks_the_node_named_by_default_metadata() {
    let graph = parse_pw_dump_metadata(
        r#"[
            {"id":41,"type":"PipeWire:Interface:Metadata","props":{"metadata.name":"default"},
             "metadata":[
                {"subject":0,"key":"default.audio.sink","value":{"name":"alsa_output.analog"}},
                {"subject":0,"key":"default.audio.source","value":{"name":"alsa_input.analog"}}
             ]},
            {"id":52,"type":"PipeWire:Interface:Node","info":{"props":{
                "media.class":"Audio/Sink","node.name":"alsa_output.analog",
                "node.description":"Built-in Audio Analog Stereo"
            }}},
            {"id":61,"type":"PipeWire:Interface:Node","info":{"props":{
                "media.class":"Audio/Sink","node.name":"alsa_output.hdmi",
                "node.description":"HDMI"
            }}},
            {"id":53,"type":"PipeWire:Interface:Node","info":{"props":{
                "media.class":"Audio/Source","node.name":"alsa_input.analog",
                "node.description":"Built-in Audio Analog Stereo"
            }}}
        ]"#,
    )
    .unwrap();
    assert_eq!(graph.default_sink.as_deref(), Some("alsa_output.analog"));
    assert_eq!(graph.default_source.as_deref(), Some("alsa_input.analog"));

    let outputs = graph_devices(&graph, DeviceKind::Output);
    assert_eq!(outputs.len(), 2);
    assert!(
        outputs
            .iter()
            .find(|device| device.id == "52")
            .unwrap()
            .is_default
    );
    assert!(
        !outputs
            .iter()
            .find(|device| device.id == "61")
            .unwrap()
            .is_default
    );

    let inputs = graph_devices(&graph, DeviceKind::Input);
    assert_eq!(inputs.len(), 1);
    assert!(inputs[0].is_default);
}

#[test]
fn duplicate_default_metadata_objects_are_rejected_as_ambiguous() {
    let graph = parse_pw_dump_metadata(
        r#"[
            {"id":41,"type":"PipeWire:Interface:Metadata","props":{"metadata.name":"default"},
             "metadata":[{"subject":0,"key":"default.audio.sink","value":{"name":"a"}}]},
            {"id":42,"type":"PipeWire:Interface:Metadata","props":{"metadata.name":"default"},
             "metadata":[{"subject":0,"key":"default.audio.sink","value":{"name":"b"}}]}
        ]"#,
    )
    .unwrap();
    assert!(graph.default_sink.is_none());
    assert!(graph.capabilities_rejected);
}

#[test]
fn device_debug_output_does_not_disclose_private_authority_name() {
    let device = Device {
        id: "52".into(),
        name: "Built-in Audio".into(),
        is_default: true,
        routes: Vec::new(),
        balance: None,
        authority_name: "alsa_output.private-hardware-identity".into(),
        authority_device_id: Some("41".into()),
        authority_route_device: Some(0),
    };
    let output = format!("{device:?}");
    assert!(output.contains("Built-in Audio"));
    assert!(output.contains("has_authority_name: true"));
    assert!(!output.contains("private-hardware-identity"));
}

#[test]
fn pipewire_json_correlates_exact_profiles_routes_and_node_identity() {
    let graph = parse_pw_dump_metadata(
            r#"[
                {"id":52,"type":"PipeWire:Interface:Node","permissions":["r","w","x","m"],"info":{"props":{
                    "media.class":"Audio/Sink","node.name":"alsa_output.analog",
                    "node.description":"Built-in Audio Analog Stereo",
                    "device.id":41,"card.profile.device":4},"params":{
                    "PropInfo":[{"id":"channelVolumes","container":"Array"}],
                    "Props":[{"channelMap":["FL","FR"],"channelVolumes":[0.4,0.8]}]
                }}},
                {"id":53,"type":"PipeWire:Interface:Node","info":{"props":{
                    "media.class":"Stream/Output/Audio","node.description":"Private Stream"}}},
                {"id":41,"type":"PipeWire:Interface:Device","info":{"props":{
                    "media.class":"Audio/Device","device.name":"alsa_card.private",
                    "device.description":"Built-in Audio"},"params":{
                    "EnumProfile":[
                        {"index":0,"name":"off","description":"Off","available":"yes"},
                        {"index":1,"name":"duplex","description":"Analog Stereo Duplex","available":"yes"},
                        {"index":2,"name":"unplugged","description":"Unplugged","available":"no"}
                    ],
                    "Profile":[{"index":1,"name":"duplex"}],
                    "EnumRoute":[
                        {"index":0,"direction":"Output","name":"speaker","description":"Speakers","available":"yes","profiles":[1],"devices":[4]},
                        {"index":1,"direction":"Output","name":"headphones","description":"Headphones","available":"unknown","profiles":[1],"devices":[4]}
                    ],
                    "Route":[{"index":0,"direction":"Output","device":4,"profile":1}]
                }}}
            ]"#,
        )
        .unwrap();
    assert_eq!(graph.hardware.len(), 1);
    assert_eq!(graph.hardware[0].device.name, "Built-in Audio");
    assert_eq!(graph.hardware[0].device.profiles.len(), 3);
    assert!(graph.hardware[0].device.profiles[0].is_active);

    let mut outputs = graph_devices(&graph, DeviceKind::Output);
    apply_graph_metadata(&mut outputs, &graph, DeviceKind::Output);
    assert_eq!(outputs[0].name, "Built-in Audio Analog Stereo");
    assert_eq!(outputs[0].routes.len(), 2);
    assert!(outputs[0].routes[0].is_active);
    assert_eq!(outputs[0].routes[0].name, "Speakers");
    assert_eq!(outputs[0].authority_device_id.as_deref(), Some("41"));
    assert_eq!(outputs[0].authority_route_device, Some(4));
    assert_eq!(
        outputs[0].balance.as_ref().map(|balance| balance.value),
        Some(50)
    );
    assert!(!graph.nodes.contains_key("53"));
    let error = parse_pw_dump_metadata("not json").unwrap_err();
    assert_eq!(error.detail(), "pw-dump returned invalid JSON");
}

#[test]
fn route_capabilities_reject_duplicate_indices_and_stale_profiles() {
    let duplicate = serde_json::json!({
        "EnumRoute": [
            {"index": 2, "direction": "Output", "name": "speaker", "devices": [4], "profiles": [1]},
            {"index": 2, "direction": "Output", "name": "headphones", "devices": [4], "profiles": [1]}
        ],
        "Route": []
    });
    assert!(parse_routes(&duplicate, 1).is_none());

    let stale = serde_json::json!({
        "EnumRoute": [
            {"index": 2, "direction": "Output", "name": "speaker", "devices": [4], "profiles": [1]}
        ],
        "Route": [
            {"index": 2, "direction": "Output", "device": 4, "profile": 3}
        ]
    });
    assert!(parse_routes(&stale, 1).is_none());
}

#[test]
fn stereo_balance_preserves_the_louder_channel_without_amplification() {
    let current = Balance {
        value: 50,
        left_volume: 400_000,
        right_volume: 800_000,
        left_first: true,
    };
    assert_eq!(balance_channel_targets(&current, 0), (800_000, 800_000));
    assert_eq!(balance_channel_targets(&current, -25), (800_000, 600_000));
    assert_eq!(balance_channel_targets(&current, 75), (200_000, 800_000));
    assert_eq!(balance_channel_targets(&current, 100), (0, 800_000));
    assert_eq!(balance_channel_targets(&current, -100), (800_000, 0));
}

#[test]
fn balance_requires_writable_exact_front_stereo_channels() {
    let base = serde_json::json!({
        "permissions": ["r", "w", "x"],
        "info": {"params": {
            "PropInfo": [{"id": "channelVolumes", "container": "Array"}],
            "Props": [{"channelMap": ["FL", "FR"], "channelVolumes": [0.5, 0.5]}]
        }}
    });
    assert_eq!(
        parse_node_balance(&base).map(|balance| balance.value),
        Some(0)
    );

    let mut read_only = base.clone();
    read_only["permissions"] = serde_json::json!(["r"]);
    assert!(parse_node_balance(&read_only).is_none());

    let mut surround = base.clone();
    surround["info"]["params"]["Props"][0]["channelMap"] = serde_json::json!(["FL", "FR", "FC"]);
    surround["info"]["params"]["Props"][0]["channelVolumes"] = serde_json::json!([0.5, 0.5, 0.5]);
    assert!(parse_node_balance(&surround).is_none());

    // Real ALSA-backed nodes carry a *second* Props entry alongside the
    // volume/mute one (an ALSA-route entry with `device`/`deviceName`
    // fields and no `channelVolumes`); balance parsing must find the right
    // entry rather than assuming the array has exactly one member.
    let mut alsa_shaped = base;
    alsa_shaped["info"]["params"]["Props"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"device": "front:1", "deviceName": "", "cardName": ""}));
    assert_eq!(
        parse_node_balance(&alsa_shaped).map(|balance| balance.value),
        Some(0)
    );
}

#[test]
fn graph_labels_require_the_exact_machine_list_node_name() {
    // `graph_devices` and `apply_graph_metadata` always read the same
    // `GraphMetadata` value now (one `pw-dump` call), so the ids and names
    // they see can never disagree there; this test exercises
    // `apply_graph_metadata`'s identity guard directly, against a `Device`
    // built independently of the graph (as could happen if a caller ever
    // passed a stale graph for a different read).
    let graph = parse_pw_dump_metadata(
        r#"[{
                "id":52,"type":"PipeWire:Interface:Node","info":{"props":{
                    "media.class":"Audio/Sink","node.name":"reused.private.node",
                    "node.description":"Wrong Hardware"
                }}
            }]"#,
    )
    .unwrap();
    let mut outputs = vec![Device {
        id: "52".into(),
        name: "current.private.node".into(),
        is_default: true,
        routes: Vec::new(),
        balance: None,
        authority_name: "current.private.node".into(),
        authority_device_id: None,
        authority_route_device: None,
    }];
    apply_graph_metadata(&mut outputs, &graph, DeviceKind::Output);
    assert_eq!(outputs[0].name, "current.private.node");
    assert!(outputs[0].routes.is_empty());
}

#[test]
fn command_output_reader_drains_after_the_capture_limit() {
    let input = std::io::Cursor::new(vec![7_u8; 32]);
    let (captured, truncated) = drain_bounded(input, 8).unwrap();
    assert_eq!(captured, vec![7_u8; 8]);
    assert!(truncated);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_volume_fixture_preserves_input_and_output() {
    let (output, input) = parse_macos_volume_settings(
        "output volume:67, input volume:44, alert volume:100, output muted:true",
    )
    .unwrap();
    assert_eq!(output.volume, 67);
    assert!(output.muted);
    assert_eq!(input.volume, 44);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_device_fixture_identifies_defaults() {
    let (outputs, inputs) = parse_macos_audio_devices(concat!(
        "Audio:\n\n",
        "        MacBook Pro Speakers:\n\n",
        "          Default Output Device: Yes\n",
        "          Output Source: MacBook Pro Speakers\n\n",
        "        MacBook Pro Microphone:\n\n",
        "          Default Input Device: Yes\n",
        "          Input Source: MacBook Pro Microphone",
    ));
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].name, "MacBook Pro Speakers");
    assert!(outputs[0].is_default);
    assert_eq!(inputs.len(), 1);
    assert!(inputs[0].is_default);
}

#[test]
fn errors_keep_operation_context() {
    let error = Error::new("read audio devices", "service unavailable");
    assert_eq!(
        error.to_string(),
        "could not read audio devices: service unavailable"
    );
}

/// A real `pw-dump` capture from the reference laptop (Built-in Audio
/// sink/source on an Intel HDA card), trimmed to the sink, source, hardware
/// device and `default` metadata objects and with device names/serials
/// sanitized. Captured read-only via
/// `ssh jacob@<reference> 'XDG_RUNTIME_DIR=/run/user/1000 pw-dump'` — no
/// volume/default/mute change was made to capture it.
#[test]
fn real_pw_dump_fixture_parses_into_the_expected_defaults_volumes_and_profile() {
    let dump = include_str!("fixtures/pw-dump-laptop.json");
    let graph = parse_pw_dump_metadata(dump).unwrap();
    assert!(!graph.capabilities_rejected);

    assert_eq!(
        graph.default_sink.as_deref(),
        Some("alsa_output.pci-0000_00_1b.0.analog-stereo")
    );
    assert_eq!(
        graph.default_source.as_deref(),
        Some("alsa_input.pci-0000_00_1b.0.analog-stereo")
    );

    let outputs = graph_devices(&graph, DeviceKind::Output);
    assert_eq!(outputs.len(), 1);
    assert!(outputs[0].is_default);
    assert_eq!(outputs[0].name, "Built-in Audio Analog Stereo");
    let output_id = outputs[0].id.clone();

    let inputs = graph_devices(&graph, DeviceKind::Input);
    assert_eq!(inputs.len(), 1);
    assert!(inputs[0].is_default);
    let input_id = inputs[0].id.clone();

    // The reference laptop's real sink was at 2% (wpctl-displayed) and the
    // source at 100%, both unmuted; channelVolumes are linear, so this
    // exercises the cubic-scale conversion against real hardware values.
    let output_node = graph
        .nodes
        .get(&output_id)
        .and_then(|node| node.level)
        .unwrap();
    assert_eq!(output_node.volume, 2);
    assert!(!output_node.muted);
    let input_node = graph
        .nodes
        .get(&input_id)
        .and_then(|node| node.level)
        .unwrap();
    assert_eq!(input_node.volume, 100);
    assert!(!input_node.muted);

    assert_eq!(graph.hardware.len(), 1);
    let hardware = &graph.hardware[0];
    assert_eq!(hardware.device.name, "Built-in Audio");
    assert!(
        hardware
            .device
            .profiles
            .iter()
            .find(|profile| profile.index == 1)
            .unwrap()
            .is_active
    );

    let mut outputs = outputs;
    apply_graph_metadata(&mut outputs, &graph, DeviceKind::Output);
    assert_eq!(outputs[0].routes.len(), 2);
    assert!(outputs[0].routes[0].is_active);
    assert_eq!(outputs[0].routes[0].name, "Speakers");
}
