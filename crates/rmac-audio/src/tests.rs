//! Focused audio service contracts.

use super::notification::*;
use super::*;

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
fn wpctl_level_parses_volume_and_mute() {
    assert_eq!(
        parse_wpctl_level("Volume: 0.72 [MUTED]"),
        Some(Level {
            volume: 72,
            muted: true
        })
    );
    assert_eq!(parse_wpctl_level("Volume: 1.5").unwrap().volume, 100);
}

#[test]
fn machine_readable_wpctl_lists_preserve_exact_identity_and_defaults() {
    let mut outputs = parse_wpctl_list(
        "61\talsa_output.hdmi\taudio/sink\t \n52\talsa_output.analog\taudio/sink\t*",
        DeviceKind::Output,
    )
    .unwrap();
    let mut inputs =
        parse_wpctl_list("53\talsa_input.analog\taudio/source\t*", DeviceKind::Input).unwrap();
    sort_devices(&mut outputs);
    sort_devices(&mut inputs);
    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0].id, "52");
    assert!(outputs[0].is_default);
    assert_eq!(outputs[0].authority_name, "alsa_output.analog");
    assert_eq!(inputs[0].authority_name, "alsa_input.analog");
    assert!(parse_wpctl_list("53\talsa_input.analog\taudio/sink\t*", DeviceKind::Input).is_err());
}

#[test]
fn machine_readable_wpctl_list_rejects_ambiguous_or_malformed_identity() {
    for invalid in [
        "52\talsa_output.analog\taudio/sink\t*\n52\talsa_output.hdmi\taudio/sink\t ",
        "52\talsa_output.analog\taudio/sink\t*\n61\talsa_output.hdmi\taudio/sink\t*",
        "52\talsa_output.analog\taudio/sink\t?",
        "0\talsa_output.analog\taudio/sink\t*",
        "52\talsa_output.analog\taudio/sink",
    ] {
        assert!(parse_wpctl_list(invalid, DeviceKind::Output).is_err());
    }
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

    let mut outputs =
        parse_wpctl_list("52\talsa_output.analog\taudio/sink\t*", DeviceKind::Output).unwrap();
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
    assert!(parse_pw_dump_metadata("not json").is_err());
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

    let mut surround = base;
    surround["info"]["params"]["Props"][0]["channelMap"] = serde_json::json!(["FL", "FR", "FC"]);
    surround["info"]["params"]["Props"][0]["channelVolumes"] = serde_json::json!([0.5, 0.5, 0.5]);
    assert!(parse_node_balance(&surround).is_none());
}

#[test]
fn graph_labels_require_the_exact_machine_list_node_name() {
    let graph = parse_pw_dump_metadata(
        r#"[{
                "id":52,"type":"PipeWire:Interface:Node","info":{"props":{
                    "media.class":"Audio/Sink","node.name":"reused.private.node",
                    "node.description":"Wrong Hardware"
                }}
            }]"#,
    )
    .unwrap();
    let mut outputs = parse_wpctl_list(
        "52\tcurrent.private.node\taudio/sink\t*",
        DeviceKind::Output,
    )
    .unwrap();
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
