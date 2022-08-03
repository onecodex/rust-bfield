use bfield::BField;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn build_bfield(n_secondaries: u8) -> BField<String> {
    let tmp_dir = tempfile::tempdir().unwrap();
    BField::create(
        tmp_dir.path(),
        "bfield",
        1_000_000,
        10,
        39,
        4,
        0.1,
        0.025,
        n_secondaries,
        false,
        String::new(),
    )
    .expect("to build")
}

fn bench_insertion(c: &mut Criterion) {
    let mut bfield = build_bfield(4);
    c.bench_function("bfield insertion", |b| {
        b.iter(|| bfield.insert(&1_u32.to_be_bytes().to_vec(), 1_u32, 0))
    });
}

fn bench_querying(c: &mut Criterion) {
    let mut bfield = build_bfield(4);

    // Identity database
    let max_value: u32 = 10_000;
    for p in 0..4 {
        for i in 0..max_value {
            bfield.insert(&i.to_be_bytes().to_vec(), i, p as usize);
        }
    }

    c.bench_function("bfield querying", |b| {
        b.iter(|| black_box(bfield.get(black_box(&10_000_i32.to_be_bytes().to_vec()))))
    });
}

criterion_group!(benches, bench_insertion, bench_querying);
criterion_main!(benches);
