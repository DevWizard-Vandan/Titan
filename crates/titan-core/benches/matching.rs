//! Matching engine benchmarks.
//!
//! Run with: cargo bench -p titan-core

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput, BenchmarkId};
use titan_core::{
    MatchingEngine, Order, OrderId, SymbolId, Side, OrderType,
    Price, Quantity,
};

fn create_engine(pool_bits: u32) -> MatchingEngine {
    MatchingEngine::new(SymbolId(1), pool_bits, Price::ZERO)
}

/// Benchmark inserting into an empty book.
fn bench_insert_empty(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_empty");
    group.throughput(Throughput::Elements(1));
    
    group.bench_function("limit_order", |b| {
        let mut engine = create_engine(20);
        let mut order_id = 0u64;
        
        b.iter(|| {
            order_id += 1;
            let order = Order::new(
                OrderId(order_id),
                SymbolId(1),
                Side::Buy,
                OrderType::Limit,
                Price::from_ticks(10000),
                Quantity(100),
                0,
            );
            black_box(engine.submit_order(order, order_id))
        })
    });
    
    group.finish();
}

/// Benchmark inserting into a book with existing orders.
fn bench_insert_deep_book(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_deep_book");
    group.throughput(Throughput::Elements(1));
    
    for depth in [100, 1000, 10000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(depth),
            &depth,
            |b, &depth| {
                let mut engine = create_engine(20);
                
                // Pre-populate book
                for i in 0..depth {
                    let order = Order::new(
                        OrderId(i as u64),
                        SymbolId(1),
                        Side::Sell,
                        OrderType::Limit,
                        Price::from_ticks(10000 + (i as u64 % 100)),
                        Quantity(100),
                        0,
                    );
                    engine.submit_order(order, i as u64);
                }
                
                let mut order_id = depth as u64;
                
                b.iter(|| {
                    order_id += 1;
                    let order = Order::new(
                        OrderId(order_id),
                        SymbolId(1),
                        Side::Buy,
                        OrderType::Limit,
                        Price::from_ticks(9990), // Won't match
                        Quantity(100),
                        0,
                    );
                    black_box(engine.submit_order(order, order_id))
                })
            },
        );
    }
    
    group.finish();
}

/// Benchmark matching a single order.
fn bench_match_single(c: &mut Criterion) {
    let mut group = c.benchmark_group("match_single");
    group.throughput(Throughput::Elements(1));
    
    group.bench_function("ioc_match", |b| {
        // Recreate engine each iteration to ensure consistent state
        b.iter_batched(
            || {
                let mut engine = create_engine(20);
                // Place a resting sell order
                let sell = Order::new(
                    OrderId(1),
                    SymbolId(1),
                    Side::Sell,
                    OrderType::Limit,
                    Price::from_ticks(10000),
                    Quantity(100),
                    0,
                );
                engine.submit_order(sell, 0);
                engine
            },
            |mut engine| {
                let buy = Order::new(
                    OrderId(2),
                    SymbolId(1),
                    Side::Buy,
                    OrderType::IOC,
                    Price::from_ticks(10000),
                    Quantity(100),
                    1,
                );
                black_box(engine.submit_order(buy, 1))
            },
            criterion::BatchSize::SmallInput,
        )
    });
    
    group.finish();
}

/// Benchmark matching against multiple resting orders.
fn bench_match_multiple(c: &mut Criterion) {
    let mut group = c.benchmark_group("match_multiple");
    group.throughput(Throughput::Elements(1));
    
    for count in [1, 5, 10] {
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |b, &count| {
                b.iter_batched(
                    || {
                        let mut engine = create_engine(20);
                        // Place multiple resting sell orders
                        for i in 0..count {
                            let sell = Order::new(
                                OrderId(i as u64),
                                SymbolId(1),
                                Side::Sell,
                                OrderType::Limit,
                                Price::from_ticks(10000),
                                Quantity(10),
                                i as u64,
                            );
                            engine.submit_order(sell, i as u64);
                        }
                        engine
                    },
                    |mut engine| {
                        let buy = Order::new(
                            OrderId(100),
                            SymbolId(1),
                            Side::Buy,
                            OrderType::IOC,
                            Price::from_ticks(10000),
                            Quantity(10 * count as u64),
                            100,
                        );
                        black_box(engine.submit_order(buy, 100))
                    },
                    criterion::BatchSize::SmallInput,
                )
            },
        );
    }
    
    group.finish();
}

/// Benchmark throughput.
fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");
    
    // Measure orders per second
    group.throughput(Throughput::Elements(10000));
    
    group.bench_function("mixed_workload", |b| {
        b.iter_batched(
            || create_engine(20),
            |mut engine| {
                // Simulate realistic workload: alternating buys and sells
                for i in 0..10000u64 {
                    let side = if i % 2 == 0 { Side::Buy } else { Side::Sell };
                    let price = 10000 + (i % 10); // 10 price levels
                    
                    let order = Order::new(
                        OrderId(i),
                        SymbolId(1),
                        side,
                        OrderType::Limit,
                        Price::from_ticks(price),
                        Quantity(100),
                        i,
                    );
                    black_box(engine.submit_order(order, i));
                }
            },
            criterion::BatchSize::SmallInput,
        )
    });
    
    group.finish();
}

criterion_group!(
    benches,
    bench_insert_empty,
    bench_insert_deep_book,
    bench_match_single,
    bench_match_multiple,
    bench_throughput,
);

criterion_main!(benches);
