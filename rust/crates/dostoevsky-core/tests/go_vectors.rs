//! The in-memory indexes against the Go engine's own results (`rust/testdata/index-misc-go.txt`).

use dostoevsky_core::admission::IndexFrequency;
use dostoevsky_core::location_index::{Location, LocationIndex};

fn list(field: &str) -> Vec<&str> {
    let inner = field.trim_start_matches('[').trim_end_matches(']');
    if inner.is_empty() {
        Vec::new()
    } else {
        inner.split(',').collect()
    }
}

#[test]
fn the_admission_sketch_and_the_location_index_match_go() {
    let (mut sketches, mut indexes, mut locations) = (0, 0, 0);
    for line in include_str!("../../../testdata/index-misc-go.txt")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "FREQ" => {
                let mut sketch = IndexFrequency::default();
                for id in list(f[1]) {
                    sketch.observe(id.parse().unwrap());
                }
                for score in list(f[2]) {
                    let (id, want) = score.split_once('=').unwrap();
                    assert_eq!(sketch.score(id.parse().unwrap()).to_string(), want, "id {id}");
                }
                sketches += 1;
            }
            "LOC" => {
                let mut index = LocationIndex::default();
                for entry in list(f[1]) {
                    let (at, size) = entry.split_once(':').unwrap();
                    index.append(Location {
                        at: at.parse().unwrap(),
                        size: size.parse().unwrap(),
                    });
                }
                let want = list(f[2]);
                assert_eq!(index.count as usize, want.len());
                for (i, entry) in want.iter().enumerate() {
                    let loc = index.location(i as i64);
                    assert_eq!(format!("{}:{}", loc.at, loc.size), *entry, "entry {i}");
                    locations += 1;
                }
                indexes += 1;
            }
            other => panic!("unknown row {other}"),
        }
    }
    assert!(
        sketches == 30 && indexes == 30 && locations > 1000,
        "{sketches} sketches, {indexes} indexes, {locations} locations"
    );
}
