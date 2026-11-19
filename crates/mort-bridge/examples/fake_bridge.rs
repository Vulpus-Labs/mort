//! A bridge over a fake session, on a socket, for trying `mort` and
//! `mort view` without REAPER (ticket 0021's speed check).
//!
//! ```text
//! cargo run --release -p mort-bridge --example fake_bridge -- 40 5
//! mort --socket <printed path> measure 1.1..151.1
//! mort --socket <printed path> view --open
//! mort --socket <printed path> play 9.1..17.1
//! ```
//!
//! Arguments: the number of tracks (default 40) and minutes (default 5).
//! Tracks are pink-noise parts in rising ranges, eight to a folder, with a
//! region every 30 s. The transport runs in real time.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use mort_bridge::dispatch::Dispatcher;
use mort_bridge::feed::Feed;
use mort_bridge::server::Server;
use mort_session::{FakeSession, NewTrack, Session, Source};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tracks: u32 = args.first().and_then(|a| a.parse().ok()).unwrap_or(40);
    let minutes: u32 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(5);
    let mut s = FakeSession::new();
    let mut folder = None;
    for i in 0..tracks {
        if i % 8 == 0 {
            folder = Some(
                s.add_track(NewTrack {
                    name: format!("Group {}", i / 8 + 1),
                    ..NewTrack::default()
                })
                .unwrap(),
            );
        }
        let t = s
            .add_track(NewTrack {
                name: format!("Part {:02}", i + 1),
                parent: folder,
                ..NewTrack::default()
            })
            .unwrap();
        let lo = 40.0 * 2f64.powf(f64::from(i % 20) * 0.4);
        s.add_sound(
            t,
            Source::Pink {
                lo,
                hi: lo * 2.5,
                db: -30.0 - f64::from(i % 3) * 4.0,
            },
            None,
        )
        .unwrap();
    }
    for r in 0..minutes * 2 {
        let start = f64::from(r) * 30.0;
        s.add_region(&format!("part {}", r + 1), start, Some(start + 30.0));
    }
    let dir = std::env::temp_dir().join("mort").join("fake");
    let (tx, rx) = mpsc::channel();
    let feed = Arc::new(Feed::new());
    let server = Server::start_with_feed(&dir, "fake", tx, Some(Arc::clone(&feed))).unwrap();
    let mut d = Dispatcher::new(rx);
    mort_bridge::methods::register_reads(&mut d);
    mort_bridge::methods::register_edits(&mut d);
    mort_bridge::methods::register_automation(&mut d);
    mort_bridge::methods::register_midi(&mut d);
    mort_bridge::methods::register_transport(&mut d);
    mort_bridge::methods::register_probes(&mut d);
    mort_bridge::measure::register_measure_with_feed(&mut d, feed);
    println!("{}", server.path().display());
    println!("{tracks} tracks, {minutes} minutes; stop with Ctrl-C");
    let mut last = Instant::now();
    loop {
        s.advance_by(last.elapsed().as_secs_f64());
        last = Instant::now();
        d.tick(&mut s, Instant::now(), Duration::from_millis(8));
        std::thread::sleep(Duration::from_millis(15));
    }
}
