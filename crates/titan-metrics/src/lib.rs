//! Latency tracking and metrics with HdrHistogram.
//!
//! Provides nanosecond-precision latency measurement.

use hdrhistogram::Histogram;

/// High-precision latency histogram.
pub struct LatencyHistogram {
    histogram: Histogram<u64>,
}

impl LatencyHistogram {
    /// Create a new histogram with 3 significant digits.
    pub fn new() -> Self {
        Self {
            histogram: Histogram::new(3).expect("Failed to create histogram"),
        }
    }
    
    /// Create with custom precision (1-5 significant digits).
    pub fn with_precision(sigfig: u8) -> Self {
        Self {
            histogram: Histogram::new(sigfig).expect("Failed to create histogram"),
        }
    }
    
    /// Record a latency value in nanoseconds.
    #[inline(always)]
    pub fn record(&mut self, nanos: u64) {
        let _ = self.histogram.record(nanos);
    }
    
    /// Get value at percentile (0.0 - 100.0).
    pub fn value_at_percentile(&self, percentile: f64) -> u64 {
        self.histogram.value_at_quantile(percentile / 100.0)
    }
    
    /// Get P50 (median) latency.
    pub fn p50(&self) -> u64 {
        self.value_at_percentile(50.0)
    }
    
    /// Get P90 latency.
    pub fn p90(&self) -> u64 {
        self.value_at_percentile(90.0)
    }
    
    /// Get P95 latency.
    pub fn p95(&self) -> u64 {
        self.value_at_percentile(95.0)
    }
    
    /// Get P99 latency.
    pub fn p99(&self) -> u64 {
        self.value_at_percentile(99.0)
    }
    
    /// Get P99.9 latency.
    pub fn p999(&self) -> u64 {
        self.value_at_percentile(99.9)
    }
    
    /// Get maximum latency.
    pub fn max(&self) -> u64 {
        self.histogram.max()
    }
    
    /// Get minimum latency.
    pub fn min(&self) -> u64 {
        self.histogram.min()
    }
    
    /// Get mean latency.
    pub fn mean(&self) -> f64 {
        self.histogram.mean()
    }
    
    /// Get standard deviation.
    pub fn stddev(&self) -> f64 {
        self.histogram.stdev()
    }
    
    /// Get total count of recorded values.
    pub fn count(&self) -> u64 {
        self.histogram.len()
    }
    
    /// Reset the histogram.
    pub fn reset(&mut self) {
        self.histogram.reset();
    }
    
    /// Print a summary of latencies.
    pub fn print_summary(&self, prefix: &str) {
        println!("{} Distribution:", prefix);
        println!("{}   P50:   {:>8} ns", prefix, self.p50());
        println!("{}   P90:   {:>8} ns", prefix, self.p90());
        println!("{}   P95:   {:>8} ns", prefix, self.p95());
        println!("{}   P99:   {:>8} ns", prefix, self.p99());
        println!("{}   P99.9: {:>8} ns", prefix, self.p999());
        println!("{}   Max:   {:>8} ns", prefix, self.max());
    }
    
    /// Format latency with appropriate units.
    pub fn format_latency(nanos: u64) -> String {
        if nanos < 1_000 {
            format!("{} ns", nanos)
        } else if nanos < 1_000_000 {
            format!("{:.2} μs", nanos as f64 / 1_000.0)
        } else if nanos < 1_000_000_000 {
            format!("{:.2} ms", nanos as f64 / 1_000_000.0)
        } else {
            format!("{:.2} s", nanos as f64 / 1_000_000_000.0)
        }
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

/// RDTSC-based timer for lowest overhead timing.
pub struct RdtscTimer {
    clock: quanta::Clock,
}

impl RdtscTimer {
    /// Create a new timer.
    pub fn new() -> Self {
        Self {
            clock: quanta::Clock::new(),
        }
    }
    
    /// Get current timestamp.
    #[inline(always)]
    pub fn now(&self) -> u64 {
        self.clock.raw()
    }
    
    /// Convert raw timestamp to nanoseconds.
    #[inline(always)]
    pub fn delta_as_nanos(&self, start: u64, end: u64) -> u64 {
        self.clock.delta_as_nanos(start, end)
    }
}

impl Default for RdtscTimer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_histogram_basic() {
        let mut h = LatencyHistogram::new();
        
        for i in 1..=100 {
            h.record(i * 100);
        }
        
        assert_eq!(h.count(), 100);
        assert!(h.p50() >= 4900 && h.p50() <= 5100);
        assert_eq!(h.min(), 100);
        // HdrHistogram may round max value slightly
        assert!(h.max() >= 10000 && h.max() <= 10100);
    }
    
    #[test]
    fn test_format_latency() {
        assert_eq!(LatencyHistogram::format_latency(500), "500 ns");
        assert_eq!(LatencyHistogram::format_latency(5000), "5.00 μs");
        assert_eq!(LatencyHistogram::format_latency(5_000_000), "5.00 ms");
    }
}

