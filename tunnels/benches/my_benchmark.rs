use std::io::Cursor;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rmp_serde::Deserializer;
use rmp_serde::Serializer;
use serde::Deserialize;
use serde::Serialize;
use tunnels::{show::Show, test_mode::stress};
use tunnels_lib::Snapshot;

fn criterion_benchmark(c: &mut Criterion) {
    let mut show = Show::new(vec![], vec![], None, false, None).unwrap();
    show.test_mode(stress);
    let frame = show.create_frame(0);
    let video_outs = frame.mixer.render(
        &frame.clocks,
        &frame.color_palette,
        &frame.positions,
        frame.audio_envelope,
    );
    println!(
        "{}, {}, {}",
        video_outs.len(),
        video_outs[0].len(),
        video_outs[0][0].len()
    );
    let snapshot = Snapshot {
        frame_number: frame.number,
        time: frame.timestamp,
        layers: video_outs[0].clone(),
    };
    let mut send_buf = Vec::new();
    snapshot
        .serialize(black_box(&mut Serializer::new(&mut send_buf)))
        .unwrap();
    c.bench_function("deserialize frame", |b| {
        b.iter(|| {
            let cur = Cursor::new(&send_buf[..]);
            let mut de = Deserializer::new(cur);
            let _typed_msg: Snapshot = Deserialize::deserialize(&mut de).unwrap();
        })
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
