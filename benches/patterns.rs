use criterion::{black_box, criterion_group, criterion_main, Criterion};
use env_logger;

use tgrep::utils::patterns::Patterns;

fn double_star(c: &mut Criterion) {
    let _ = env_logger::builder().try_init();
    let patterns = Patterns::new("/", &vec!["foo/bar/**/qux/xyz".to_string()]);
    c.bench_function("patters", |b| {
        b.iter(|| {
            patterns.is_excluded(black_box("foo/bar/zoo/too/qux/xyz"), false);
        })
    });
}

criterion_group!(benches, double_star);
criterion_main!(benches);
