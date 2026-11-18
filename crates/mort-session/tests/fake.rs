//! `FakeSession` behaves as the bridge expects REAPER to (ADR 0011 §1).

#![expect(clippy::float_cmp, reason = "values compared are stored, not computed")]

use mort_proto::model::{ControlChange, EnvelopePoint, EnvelopeTarget, Note, Shape};
use mort_proto::{ErrorKind, Guid, TrackId};
use mort_session::{FakeSession, NewTrack, SendChange, Session, TrackChange};

fn track(s: &mut FakeSession, name: &str) -> TrackId {
    s.add_track(NewTrack {
        name: name.into(),
        ..NewTrack::default()
    })
    .unwrap()
}

fn kind<T: std::fmt::Debug>(r: mort_session::Result<T>) -> ErrorKind {
    r.unwrap_err().kind().expect("a mort error")
}

#[test]
fn state_moves_on_every_edit() {
    let mut s = FakeSession::new();
    let mut last = s.state();
    let mut step = |s: &FakeSession| {
        assert!(s.state() > last, "state did not move");
        last = s.state();
    };
    let bass = track(&mut s, "bass");
    step(&s);
    let pad = track(&mut s, "pad");
    step(&s);
    s.set_track(
        bass,
        &TrackChange {
            volume: Some(0.5),
            ..TrackChange::default()
        },
    )
    .unwrap();
    step(&s);
    s.add_send(bass, pad).unwrap();
    step(&s);
    s.set_send(
        bass,
        0,
        &SendChange {
            mute: Some(true),
            ..SendChange::default()
        },
    )
    .unwrap();
    step(&s);
    let eq = s.add_fx(bass, "ReaEQ", None).unwrap();
    step(&s);
    s.set_param(eq, 1, 0.25).unwrap();
    step(&s);
    s.set_fx_enabled(eq, false).unwrap();
    step(&s);
    let env = s
        .envelope(EnvelopeTarget::Volume { track: bass }, true)
        .unwrap()
        .unwrap();
    step(&s);
    s.insert_envelope_points(
        env.id,
        &[EnvelopePoint {
            time: 1.0,
            value: 0.5,
            shape: Shape::Slow,
            tension: 0.0,
        }],
    )
    .unwrap();
    step(&s);
    let item = s.add_midi_item(bass, 0.0, 4.0).unwrap();
    step(&s);
    s.insert_midi(
        item,
        &[Note {
            pitch: 36,
            start: 0.0,
            end: 0.5,
            velocity: 100,
            channel: 0,
        }],
        &[],
    )
    .unwrap();
    step(&s);
    s.remove_fx(eq).unwrap();
    step(&s);
    s.remove_track(pad).unwrap();
    step(&s);
}

#[test]
fn reads_do_not_move_state() {
    let mut s = FakeSession::new();
    let bass = track(&mut s, "bass");
    let eq = s.add_fx(bass, "ReaEQ", None).unwrap();
    let before = s.state();
    let _ = (
        s.tracks(),
        s.track(bass),
        s.params(eq),
        s.track_chunk(bass),
        s.items(bass),
    );
    assert_eq!(
        s.envelope(EnvelopeTarget::Pan { track: bass }, false)
            .unwrap(),
        None
    );
    assert_eq!(s.state(), before);
}

#[test]
fn chunks_change_exactly_when_the_track_does() {
    let mut s = FakeSession::new();
    let bass = track(&mut s, "bass");
    let pad = track(&mut s, "pad");
    let (b0, p0) = (s.track_chunk(bass).unwrap(), s.track_chunk(pad).unwrap());
    assert_eq!(s.track_chunk(bass).unwrap(), b0, "stable");

    let eq = s.add_fx(bass, "ReaEQ", None).unwrap();
    let b1 = s.track_chunk(bass).unwrap();
    assert_ne!(b1, b0);
    assert_eq!(s.track_chunk(pad).unwrap(), p0, "other track untouched");

    s.set_param(eq, 0, 0.3).unwrap();
    let b2 = s.track_chunk(bass).unwrap();
    assert_ne!(b2, b1);
    s.set_param(eq, 0, 0.5).unwrap();
    assert_eq!(
        s.track_chunk(bass).unwrap(),
        b1,
        "same contents, same chunk"
    );
}

#[test]
fn undo_points_have_labels_and_restore() {
    let mut s = FakeSession::new();
    let bass = track(&mut s, "bass");

    s.begin_undo();
    let eq = s.add_fx(bass, "ReaEQ", None).unwrap();
    s.set_param(eq, 1, 0.25).unwrap();
    s.end_undo("mort: bass EQ");
    let with_eq = s.track_chunk(bass).unwrap();

    s.begin_undo();
    s.end_undo("mort: nothing");
    assert_eq!(
        s.undo_label().as_deref(),
        Some("mort: bass EQ"),
        "empty block leaves no point"
    );

    let before = s.state();
    assert_eq!(s.undo().as_deref(), Some("mort: bass EQ"));
    assert!(s.state() > before);
    assert!(s.track(bass).unwrap().fx.is_empty());
    assert_eq!(kind(s.params(eq)), ErrorKind::NotFound);

    assert_eq!(s.redo().as_deref(), Some("mort: bass EQ"));
    assert_eq!(s.track_chunk(bass).unwrap(), with_eq);
    assert_eq!(s.params(eq).unwrap()[1].value, 0.25);

    s.begin_undo();
    s.set_track(
        bass,
        &TrackChange {
            mute: Some(true),
            ..TrackChange::default()
        },
    )
    .unwrap();
    s.end_undo("mort: mute bass");
    assert_eq!(s.redo(), None, "a new point clears redo");
}

#[test]
fn errors_name_the_element() {
    let mut s = FakeSession::new();
    let ghost = TrackId(Guid(42));
    let e = s.track(ghost).unwrap_err();
    assert_eq!(e.kind(), Some(ErrorKind::NotFound));
    assert_eq!(
        e.data.as_ref().unwrap().subject.as_deref(),
        Some(ghost.to_string().as_str())
    );

    let bass = track(&mut s, "bass");
    let e = s.add_fx(bass, "Serum", None).unwrap_err();
    assert!(
        e.data.unwrap().fix.contains("ReaEQ"),
        "fix lists the plugins"
    );

    let eq = s.add_fx(bass, "reaeq", None).unwrap();
    assert_eq!(kind(s.set_param(eq, 9, 0.5)), ErrorKind::NotFound);
    assert_eq!(kind(s.set_param(eq, 0, 1.5)), ErrorKind::Invalid);
    assert_eq!(kind(s.remove_track(s.master())), ErrorKind::Invalid);
    assert_eq!(
        kind(s.set_track(
            bass,
            &TrackChange {
                pan: Some(2.0),
                ..TrackChange::default()
            }
        )),
        ErrorKind::Invalid
    );
    assert_eq!(kind(s.add_send(bass, bass)), ErrorKind::Invalid);
    assert_eq!(kind(s.load_preset(eq, "Nope")), ErrorKind::NotFound);
}

#[test]
fn params_display_and_presets() {
    let mut s = FakeSession::new();
    let bass = track(&mut s, "bass");
    let eq = s.add_fx(bass, "ReaEQ", None).unwrap();
    let p = s.params(eq).unwrap();
    assert_eq!(p[0].name, "Freq-Band 1");
    assert_eq!(p[0].display, "632 Hz");
    assert_eq!(p[1].display, "0.00 dB");
    assert_eq!(
        s.format_param(eq, 0, 1.0).unwrap().as_deref(),
        Some("20000 Hz")
    );
    assert_eq!(s.params(eq).unwrap()[0].value, 0.5, "format does not set");

    s.load_preset(eq, "Mud cut").unwrap();
    assert_eq!(s.params(eq).unwrap()[1].display, "-3.00 dB");

    let st = s.fx_state(eq).unwrap();
    s.set_param(eq, 1, 1.0).unwrap();
    s.set_fx_state(eq, &st).unwrap();
    assert_eq!(s.params(eq).unwrap()[1].display, "-3.00 dB");
    assert_eq!(kind(s.set_fx_state(eq, "junk")), ErrorKind::Invalid);
}

#[test]
fn fx_chain_order() {
    let mut s = FakeSession::new();
    let bass = track(&mut s, "bass");
    let eq = s.add_fx(bass, "ReaEQ", None).unwrap();
    let comp = s.add_fx(bass, "ReaComp", Some(0)).unwrap();
    let probe = s.add_fx(bass, "mort probe", None).unwrap();
    let names = |s: &FakeSession| {
        s.track(bass)
            .unwrap()
            .fx
            .iter()
            .map(|f| f.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&s), [comp, eq, probe]);
    s.move_fx(probe, 0).unwrap();
    assert_eq!(names(&s), [probe, comp, eq]);
    let (t, info) = s.fx(eq).unwrap();
    assert_eq!(
        (t, info.index, info.name.as_str()),
        (bass, 2, "ReaEQ (Cockos)")
    );
    assert_eq!(kind(s.move_fx(eq, 3)), ErrorKind::Invalid);
}

#[test]
fn removing_a_track_cleans_up() {
    let mut s = FakeSession::new();
    let folder = track(&mut s, "drums");
    let kick = s
        .add_track(NewTrack {
            name: "kick".into(),
            parent: Some(folder),
            ..NewTrack::default()
        })
        .unwrap();
    let verb = track(&mut s, "verb");
    let delay = track(&mut s, "delay");
    s.add_send(kick, verb).unwrap();
    s.add_send(kick, delay).unwrap();
    s.envelope(
        EnvelopeTarget::SendVolume {
            track: kick,
            send: 0,
        },
        true,
    )
    .unwrap();
    let delay_env = s
        .envelope(
            EnvelopeTarget::SendVolume {
                track: kick,
                send: 1,
            },
            true,
        )
        .unwrap()
        .unwrap();

    s.remove_track(verb).unwrap();
    let k = s.track(kick).unwrap();
    assert_eq!(k.sends.len(), 1);
    assert_eq!(k.sends[0].dest, delay);
    let envs = s.envelopes(kick).unwrap();
    assert_eq!(envs.len(), 1, "envelope of the removed send went");
    assert_eq!(envs[0].id, delay_env.id);
    assert_eq!(
        envs[0].target,
        EnvelopeTarget::SendVolume {
            track: kick,
            send: 0
        },
        "index shifted down"
    );

    s.remove_track(folder).unwrap();
    assert_eq!(s.track(kick).unwrap().parent, None, "children move up");
    assert_eq!(
        s.tracks().iter().map(|t| t.index).collect::<Vec<_>>(),
        [Some(0), Some(1)]
    );
    assert_eq!(s.track(s.master()).unwrap().index, None);
}

#[test]
fn envelopes() {
    let mut s = FakeSession::new();
    let bass = track(&mut s, "bass");
    let eq = s.add_fx(bass, "ReaEQ", None).unwrap();
    let target = EnvelopeTarget::Param { fx: eq, index: 0 };
    let env = s.envelope(target, true).unwrap().unwrap();
    assert_eq!(
        s.envelope(target, true).unwrap().unwrap().id,
        env.id,
        "found, not recreated"
    );
    let pt = |time, value| EnvelopePoint {
        time,
        value,
        shape: Shape::Linear,
        tension: 0.0,
    };
    s.insert_envelope_points(env.id, &[pt(4.0, 0.8), pt(0.0, 0.2), pt(2.0, 0.5)])
        .unwrap();
    let all = s.envelope_points(env.id, 0.0, 10.0).unwrap();
    assert_eq!(
        all.iter().map(|p| p.time).collect::<Vec<_>>(),
        [0.0, 2.0, 4.0]
    );
    assert_eq!(
        s.envelope_points(env.id, 2.0, 4.0).unwrap().len(),
        1,
        "end exclusive"
    );
    s.delete_envelope_points(env.id, 1.0, 3.0).unwrap();
    assert_eq!(s.envelope_points(env.id, 0.0, 10.0).unwrap().len(), 2);
    assert_eq!(
        kind(s.insert_envelope_points(env.id, &[pt(1.0, 1.5)])),
        ErrorKind::Invalid
    );
    assert_eq!(
        kind(s.envelope(EnvelopeTarget::Param { fx: eq, index: 7 }, true)),
        ErrorKind::NotFound
    );

    s.remove_fx(eq).unwrap();
    assert!(
        s.envelopes(bass).unwrap().is_empty(),
        "parameter envelopes go with their FX"
    );
}

#[test]
fn midi() {
    let mut s = FakeSession::new();
    let bass = track(&mut s, "bass");
    let item = s.add_midi_item(bass, 2.0, 6.0).unwrap();
    let n = |pitch, start, end| Note {
        pitch,
        start,
        end,
        velocity: 100,
        channel: 0,
    };
    let cc = ControlChange {
        time: 3.0,
        controller: 1,
        value: 64,
        channel: 0,
    };
    s.insert_midi(item, &[n(40, 4.0, 5.0), n(36, 2.0, 3.0)], &[cc])
        .unwrap();
    assert_eq!(
        s.notes(item)
            .unwrap()
            .iter()
            .map(|n| n.pitch)
            .collect::<Vec<_>>(),
        [36, 40]
    );
    assert_eq!(s.control_changes(item).unwrap(), [cc]);
    assert_eq!(
        kind(s.insert_midi(item, &[n(1, 5.5, 6.5)], &[])),
        ErrorKind::Invalid
    );
    s.delete_midi(item, 3.0, 6.0).unwrap();
    assert_eq!(s.notes(item).unwrap().len(), 1);
    assert!(s.control_changes(item).unwrap().is_empty());
    let items = s.items(bass).unwrap();
    assert_eq!(
        (items[0].position, items[0].length, items[0].midi),
        (2.0, 4.0, true)
    );
    assert_eq!(s.project().length, 6.0);
    assert_eq!(kind(s.add_midi_item(bass, 3.0, 3.0)), ErrorKind::Invalid);
}

#[test]
fn transport() {
    let mut s = FakeSession::new();
    s.play(2.0, 4.0).unwrap();
    assert!(s.is_playing());
    s.advance_to(3.0);
    assert_eq!(s.position(), 3.0);
    s.advance_to(5.0);
    assert!(!s.is_playing());
    assert_eq!(s.position(), 4.0);
    assert_eq!(kind(s.play(4.0, 2.0)), ErrorKind::Invalid);
}

#[test]
fn regions_and_tempo() {
    let mut s = FakeSession::new();
    s.add_region("chorus", 16.0, Some(32.0));
    s.add_region("verse", 0.0, Some(16.0));
    s.add_region("drop", 24.0, None);
    let names: Vec<_> = s.regions().into_iter().map(|r| r.name).collect();
    assert_eq!(names, ["verse", "chorus", "drop"]);
    let pos: mort_proto::Position = "9.1".parse().unwrap();
    assert_eq!(pos.resolve(s.tempo_map(), None, "9.1").unwrap(), 16.0);
}
