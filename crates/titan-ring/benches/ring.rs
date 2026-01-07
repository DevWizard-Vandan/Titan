//! Ring buffer benchmarks.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use titan_ring::SpscRing;

fn bench_publish_consume(c: &mut Criterion) {
    let mut group = c.benchmark_group("ring_buffer");
    group.throughput(Throughput::Elements(1));
    
    group.bench_function("try_publish", |b| {
        let mut ring: SpscRing<u64, 1024> = SpscRing::new();
        let (mut producer, mut consumer) = ring.split();
        
        b.iter(|| {
            black_box(producer.try_publish(42));
            black_box(consumer.try_consume());
        })
    });
    
    group.finish();
}

fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("ring_throughput");
    group.throughput(Throughput::Elements(10000));
    
    group.bench_function("10k_messages", |b| {
        b.iter_batched(
            || {
                let ring: SpscRing<u64, 16384> = SpscRing::new();
                ring
            },
            |mut ring| {
                let (mut producer, mut consumer) = ring.split();
                for i in 0..10000u64 {
                    producer.publish(i);
                }
                for _ in 0..10000 {
                    black_box(consumer.consume());
                }
            },
            criterion::BatchSize::SmallInput,
        )
    });
    
    group.finish();
}

criterion_group!(benches, bench_publish_consume, bench_throughput);
criterion_main!(benches);
