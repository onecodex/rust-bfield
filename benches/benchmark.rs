use bfield::BField;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn build_bfield(n_secondaries: u8, max_value: u32) -> BField<String> {
    let mut tmp_dir = tempfile::tempdir().unwrap();
    let mut bfield = BField::create(
        tmp_dir.path(),
        "bfield",
        1_000_000,
        10,
        39,
        4,
        0.1,
        0.025,
        n_secondaries,
        String::new(),
    )
    .expect("to build");

    // Identity database
    for p in 0..n_secondaries {
        for i in 0..max_value {
            bfield.insert(&i.to_be_bytes().to_vec(), i, p as usize);
        }
    }

    bfield
}

fn bench_creation(c: &mut Criterion) {
    c.bench_function("bfield creation", |b| {
        b.iter(|| build_bfield(black_box(4), black_box(10_000)))
    });
}

fn bench_querying(c: &mut Criterion) {
    let bfield = build_bfield(4, 10_000);
    c.bench_function("bfield querying", |b| {
        b.iter(|| black_box(bfield.get(black_box(&10_000_i32.to_be_bytes().to_vec()))))
    });
}

criterion_group!(benches, bench_creation, bench_querying);
criterion_main!(benches);
