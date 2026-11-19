//! Captures, replacement, staleness and the sidecar (ADR 0006).

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    reason = "test fixtures"
)]

use std::collections::BTreeMap;

use mort_map::{Capture, Frame, MixMap, Now, Routing, Source, Span, Stale, TimedFrame, store};
use mort_proto::{Guid, SteppedTempoMap, TrackId};

const MASTER: TrackId = TrackId(Guid(1));
const DRUMS: TrackId = TrackId(Guid(2));
const KICK: TrackId = TrackId(Guid(3));
const BASS: TrackId = TrackId(Guid(4));
const VERB: TrackId = TrackId(Guid(5));

/// Kick in the Drums folder; Drums, Bass and Verb to the master; Bass
/// sends to Verb.
fn routing() -> Routing {
    let mut r = Routing::new();
    r.connect(KICK, DRUMS);
    r.connect(DRUMS, MASTER);
    r.connect(BASS, MASTER);
    r.connect(VERB, MASTER);
    r.connect(BASS, VERB);
    r
}

fn hashes(changed: &[TrackId]) -> BTreeMap<TrackId, u64> {
    [MASTER, DRUMS, KICK, BASS, VERB]
        .into_iter()
        .map(|t| (t, if changed.contains(&t) { 99 } else { 1 }))
        .collect()
}

fn frames(span: Span, tag: u32) -> Vec<TimedFrame> {
    // Frames every 50 ms from the start, running 150 ms past the end.
    let n = ((span.end - span.start + 0.15) * 20.0).round() as u32;
    (0..n)
        .map(|i| TimedFrame {
            start: span.start + f64::from(i) / 20.0,
            frame: Frame {
                index: tag,
                ..Frame::default()
            },
        })
        .collect()
}

fn capture(
    map: &mut MixMap,
    tempo: &SteppedTempoMap,
    span: Span,
    tracks: &[TrackId],
    tag: u32,
) -> u64 {
    let r = routing();
    let c = Capture {
        id: 0,
        span,
        taken: 0,
        state: 1,
        source: Source::Render,
        file: None,
        tempo: mort_map::tempo_fingerprint(tempo, span.end),
        hashes: hashes(&[]),
        inputs: tracks
            .iter()
            .map(|t| (*t, r.upstream(*t).into_iter().collect()))
            .collect(),
        tracks: vec![],
    };
    let fs = tracks.iter().map(|t| (*t, frames(span, tag))).collect();
    map.add(c, fs).id
}

fn now<'a>(tempo: &'a SteppedTempoMap, changed: &[TrackId]) -> Now<'a> {
    Now {
        hashes: hashes(changed),
        routing: routing(),
        tempo,
    }
}

const ALL: [TrackId; 5] = [MASTER, DRUMS, KICK, BASS, VERB];

#[test]
fn routing_walks_both_ways() {
    let r = routing();
    assert_eq!(
        r.downstream(KICK).into_iter().collect::<Vec<_>>(),
        [MASTER, DRUMS]
    );
    assert_eq!(
        r.downstream(BASS).into_iter().collect::<Vec<_>>(),
        [MASTER, VERB]
    );
    assert_eq!(r.upstream(VERB).into_iter().collect::<Vec<_>>(), [BASS]);
    assert_eq!(r.upstream(MASTER).len(), 4);
}

#[test]
fn a_capture_clips_and_replaces_older_frames() {
    let tempo = SteppedTempoMap::constant(120.0, 4, 4);
    let mut m = MixMap::new();
    let a = capture(&mut m, &tempo, Span::new(0.0, 10.0), &ALL, 1);
    assert_eq!(
        m.segments(KICK)[0].frames.len(),
        200,
        "the render's tail is clipped"
    );

    // Re-measure the kick and its buses over 4–6 s.
    let b = capture(
        &mut m,
        &tempo,
        Span::new(4.0, 6.0),
        &[KICK, DRUMS, MASTER],
        2,
    );
    let k = m.segments(KICK);
    let spans: Vec<(f64, f64, u64)> = k
        .iter()
        .map(|s| (s.span.start, s.span.end, s.capture))
        .collect();
    assert_eq!(spans, [(0.0, 4.0, a), (4.0, 6.0, b), (6.0, 10.0, a)]);
    assert_eq!(
        k[0].frames.len() + k[1].frames.len() + k[2].frames.len(),
        200
    );
    assert!(k[1].frames.iter().all(|f| f.frame.index == 2));
    assert_eq!(m.segments(BASS).len(), 1, "bass was not re-measured");

    // A capture over everything a and b cover supersedes them; they are
    // kept for comparing.
    let added = {
        let r = routing();
        let span = Span::new(0.0, 10.0);
        m.add(
            Capture {
                id: 0,
                span,
                taken: 0,
                state: 2,
                source: Source::Render,
                file: None,
                tempo: mort_map::tempo_fingerprint(&tempo, 10.0),
                hashes: hashes(&[]),
                tracks: vec![],
                inputs: ALL
                    .iter()
                    .map(|t| (*t, r.upstream(*t).into_iter().collect()))
                    .collect(),
            },
            ALL.iter().map(|t| (*t, frames(span, 3))).collect(),
        )
    };
    assert_eq!(added.superseded, [a, b]);
    assert!(added.dropped.is_empty());
    assert_eq!(m.captures().count(), 1);
    assert_eq!(m.all_captures().count(), 3);

    // As it was after b: a's frames, with b's over 4–6 s on the kick.
    let then = m.as_of(b, Span::new(0.0, 10.0));
    let k = then.segments(KICK);
    let spans: Vec<(f64, f64, u64)> = k
        .iter()
        .map(|s| (s.span.start, s.span.end, s.capture))
        .collect();
    assert_eq!(spans, [(0.0, 4.0, a), (4.0, 6.0, b), (6.0, 10.0, a)]);
    assert!(k[1].frames.iter().all(|f| f.frame.index == 2));
    assert_eq!(then.segments(BASS)[0].capture, a);
    let early = m.as_of(a, Span::new(3.0, 5.0));
    assert_eq!(early.segments(KICK).len(), 1);
    assert_eq!(early.segments(KICK)[0].frames.len(), 40);
}

#[test]
fn editing_one_track_makes_it_and_its_buses_stale_and_nothing_else() {
    let tempo = SteppedTempoMap::constant(120.0, 4, 4);
    let mut m = MixMap::new();
    capture(&mut m, &tempo, Span::new(0.0, 8.0), &ALL, 1);
    assert!(m.stale(&now(&tempo, &[]), None).is_empty());

    let stale = m.stale(&now(&tempo, &[KICK]), None);
    assert_eq!(
        stale.keys().copied().collect::<Vec<_>>(),
        [MASTER, DRUMS, KICK]
    );
    let cov = m.coverage(&now(&tempo, &[KICK]));
    assert_eq!(cov[&DRUMS][0].stale, Some(Stale::Changed { track: KICK }));

    let stale = m.stale(&now(&tempo, &[BASS]), Some(Span::new(2.0, 3.0)));
    assert_eq!(
        stale.keys().copied().collect::<Vec<_>>(),
        [MASTER, BASS, VERB]
    );
    assert_eq!(stale[&VERB], [Span::new(2.0, 3.0)]);

    // A bus's own change does not reach what feeds it.
    let stale = m.stale(&now(&tempo, &[DRUMS]), None);
    assert_eq!(stale.keys().copied().collect::<Vec<_>>(), [MASTER, DRUMS]);
}

#[test]
fn routing_and_tempo_changes() {
    let tempo = SteppedTempoMap::constant(120.0, 4, 4);
    let mut m = MixMap::new();
    capture(&mut m, &tempo, Span::new(0.0, 8.0), &ALL, 1);

    // Kick now also sends to Verb.
    let mut r = routing();
    r.connect(KICK, VERB);
    let n = Now {
        hashes: hashes(&[]),
        routing: r,
        tempo: &tempo,
    };
    assert_eq!(
        m.coverage(&n)[&VERB][0].stale,
        Some(Stale::Routing { track: KICK })
    );
    assert!(m.coverage(&n)[&BASS][0].stale.is_none());

    let slower = SteppedTempoMap::constant(100.0, 4, 4);
    let stale = m.stale(&now(&slower, &[]), None);
    assert_eq!(stale.len(), 5);
    assert_eq!(
        m.coverage(&now(&slower, &[]))[&BASS][0].stale,
        Some(Stale::Tempo)
    );
}

#[test]
fn the_sidecar_round_trips() {
    let tempo = SteppedTempoMap::constant(120.0, 4, 4);
    let mut m = MixMap::new();
    let a = capture(&mut m, &tempo, Span::new(0.0, 4.0), &ALL, 1);
    let dir = std::env::temp_dir().join(format!("mort-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    store::save(&dir, &mut m, true).unwrap();
    assert!(m.unsaved().is_empty() && !m.changed());
    let back = store::load(&dir).unwrap().unwrap();
    assert_eq!(back, m);

    // Replacing the whole of capture a keeps it, until it falls out of
    // the history.
    let b = capture(&mut m, &tempo, Span::new(0.0, 4.0), &ALL, 2);
    store::save(&dir, &mut m, false).unwrap();
    assert!(dir.join(format!("{a}.frames")).exists());
    assert!(dir.join(format!("{b}.frames")).exists());
    assert_eq!(store::load(&dir).unwrap().unwrap(), m);
    for i in 0..mort_map::HISTORY {
        capture(
            &mut m,
            &tempo,
            Span::new(0.0, 4.0),
            &ALL,
            3 + u32::try_from(i).unwrap(),
        );
    }
    store::save(&dir, &mut m, false).unwrap();
    assert!(
        !dir.join(format!("{a}.frames")).exists(),
        "a fell out of the history"
    );
    assert!(dir.join(format!("{b}.frames")).exists());
    assert_eq!(store::load(&dir).unwrap().unwrap(), m);
    assert!(store::load(&dir.join("none")).unwrap().is_none());
    std::fs::remove_dir_all(&dir).unwrap();

    assert_eq!(
        store::dir_for(std::path::Path::new("/songs/fen.rpp")),
        std::path::Path::new("/songs/fen.mort/captures")
    );
}

#[test]
fn a_format_2_sidecar_still_loads() {
    let dir = std::env::temp_dir().join(format!("mort-map-v2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let kick = "{00000000-0000-0000-0000-000000000003}";
    let index = format!(
        r#"{{"version": 2, "next": 2,
            "captures": [{{"id": 1, "span": {{"start": 0.0, "end": 1.0}}, "taken": 5, "state": 1,
                          "mode": "play", "tempo": 0, "hashes": {{}}, "inputs": {{}}}}],
            "covers": {{"{kick}": [{{"capture": 1, "start": 0.0, "end": 1.0}}]}}}}"#
    );
    std::fs::write(dir.join("index.json"), index).unwrap();
    let mut bytes = b"MORTFRM2".to_vec();
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(3u128.to_le_bytes());
    bytes.extend(2u32.to_le_bytes());
    for i in 0..2u32 {
        bytes.extend((f64::from(i) * 0.05).to_le_bytes());
        bytes.extend(
            Frame {
                index: i,
                ..Frame::default()
            }
            .to_le_bytes(),
        );
    }
    std::fs::write(dir.join("1.frames"), bytes).unwrap();
    let m = store::load(&dir).unwrap().unwrap();
    assert_eq!(m.capture(1).unwrap().source, Source::Play);
    assert_eq!(m.segments(KICK)[0].frames.len(), 2);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn slivers_between_playbacks_are_dropped() {
    let tempo = SteppedTempoMap::constant(120.0, 4, 4);
    let mut m = MixMap::new();
    capture(&mut m, &tempo, Span::new(0.0, 4.0), &[KICK], 1);
    // A second playback whose grid sits 3 µs later leaves 0–0.000003 of
    // the first: no sliver may remain.
    capture(&mut m, &tempo, Span::new(0.000_003, 2.0), &[KICK], 2);
    let spans: Vec<(f64, f64)> = m
        .segments(KICK)
        .iter()
        .map(|s| (s.span.start, s.span.end))
        .collect();
    assert_eq!(spans.len(), 2, "{spans:?}");
    assert!(spans[0].1 - spans[0].0 > 1.9);
}
