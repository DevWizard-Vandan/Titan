//! Fixed-point arithmetic for prices and quantities.
//!
//! Using fixed-point avoids IEEE 754 floating-point rounding errors
//! and provides deterministic arithmetic across all platforms.

use core::ops::{Add, Sub, Mul};

/// Fixed-point price representation.
///
/// Internally stores price as integer ticks.
/// Example: $123.45 with TICK_SIZE=100 → Price(12345)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Price(pub u64);

impl Price {
    /// Minimum price increment (tick size).
    /// For $0.01 ticks, use 100 (2 decimal places).
    /// For 8 decimal places (crypto), use 100_000_000.
    pub const TICK_SIZE: u64 = 100;
    
    /// Number of decimal places.
    pub const DECIMAL_PLACES: u32 = 2;
    
    /// Zero price.
    pub const ZERO: Self = Self(0);
    
    /// Maximum price.
    pub const MAX: Self = Self(u64::MAX);
    
    /// Create a price from a number of ticks.
    #[inline(always)]
    pub const fn from_ticks(ticks: u64) -> Self {
        Self(ticks.saturating_mul(Self::TICK_SIZE))
    }
    
    /// Convert price to number of ticks.
    #[inline(always)]
    pub const fn to_ticks(self) -> u64 {
        self.0 / Self::TICK_SIZE
    }
    
    /// Get raw internal value.
    #[inline(always)]
    pub const fn as_raw(self) -> u64 {
        self.0
    }
    
    /// Create from raw value (no conversion).
    #[inline(always)]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
    
    /// Check if price is zero.
    #[inline(always)]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
    
    /// Saturating addition.
    #[inline(always)]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
    
    /// Saturating subtraction.
    #[inline(always)]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}

impl Add for Price {
    type Output = Self;
    
    #[inline(always)]
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl Sub for Price {
    type Output = Self;
    
    #[inline(always)]
    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

/// Quantity in base units (shares, contracts, satoshis, etc.).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Quantity(pub u64);

impl Quantity {
    /// Zero quantity.
    pub const ZERO: Self = Self(0);
    
    /// Maximum quantity.
    pub const MAX: Self = Self(u64::MAX);
    
    /// Check if quantity is zero.
    #[inline(always)]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
    
    /// Get raw value.
    #[inline(always)]
    pub const fn as_raw(self) -> u64 {
        self.0
    }
    
    /// Create from raw value.
    #[inline(always)]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
    
    /// Saturating addition.
    #[inline(always)]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
    
    /// Saturating subtraction.
    #[inline(always)]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
    
    /// Checked subtraction.
    #[inline(always)]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }
    
    /// Minimum of two quantities.
    #[inline(always)]
    pub const fn min(self, other: Self) -> Self {
        if self.0 < other.0 { self } else { other }
    }
}

impl Add for Quantity {
    type Output = Self;
    
    #[inline(always)]
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl Sub for Quantity {
    type Output = Self;
    
    #[inline(always)]
    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

impl Mul for Quantity {
    type Output = Self;
    
    #[inline(always)]
    fn mul(self, other: Self) -> Self {
        Self(self.0 * other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_price_ticks() {
        let p = Price::from_ticks(100);
        assert_eq!(p.to_ticks(), 100);
        assert_eq!(p.as_raw(), 100 * Price::TICK_SIZE);
    }
    
    #[test]
    fn test_quantity_ops() {
        let q1 = Quantity(100);
        let q2 = Quantity(50);
        
        assert_eq!((q1 + q2).0, 150);
        assert_eq!((q1 - q2).0, 50);
        assert_eq!(q1.min(q2), q2);
    }
    
    #[test]
    fn test_saturating_ops() {
        let q = Quantity(10);
        assert_eq!(q.saturating_sub(Quantity(20)), Quantity::ZERO);
    }
}
