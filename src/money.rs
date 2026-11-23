// SPDX-License-Identifier: AGPL-3.0-or-later
//! Money is integer rupiah. Rates are basis points (1000 = 10%). No floats, no silent rounding.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaxRules {
    pub tax_bp: i64,
    pub service_bp: i64,
    pub tax_inclusive: bool,
    pub service_in_tax_base: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Totals {
    pub subtotal: i64,
    pub discount: i64,
    pub service: i64,
    pub tax: i64,
    /// Cash rounding as its own line (never silent). Included in `total`.
    pub rounding: i64,
    pub total: i64,
}

/// Round half up on non-negative integers.
fn mul_bp(amount: i64, bp: i64) -> i64 {
    (amount * bp + 5_000) / 10_000
}

/// `subtotal` and `discount` are rupiah; `round_to` is 0 (off), 100, 500 or 1000.
pub fn compute(subtotal: i64, discount: i64, r: TaxRules, round_to: i64) -> Totals {
    let discount = discount.clamp(0, subtotal);
    let net = subtotal - discount;
    let service = mul_bp(net, r.service_bp);
    let (tax, raw_total) = if r.tax_inclusive {
        // Prices already contain tax: extract it, never add it.
        let gross = if r.service_in_tax_base { net + service } else { net };
        let tax = gross - (gross * 10_000 + (10_000 + r.tax_bp) / 2) / (10_000 + r.tax_bp);
        (tax, net + service)
    } else {
        let base = if r.service_in_tax_base { net + service } else { net };
        let tax = mul_bp(base, r.tax_bp);
        (tax, net + service + tax)
    };
    let total = if round_to > 0 {
        (raw_total + round_to / 2) / round_to * round_to
    } else {
        raw_total
    };
    Totals {
        subtotal,
        discount,
        service,
        tax,
        rounding: total - raw_total,
        total,
    }
}

/// `92400` -> `Rp92.400`
pub fn rupiah(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    format!("{}Rp{}", if n < 0 { "-" } else { "" }, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(tax_bp: i64, service_bp: i64, inclusive: bool, in_base: bool) -> TaxRules {
        TaxRules {
            tax_bp,
            service_bp,
            tax_inclusive: inclusive,
            service_in_tax_base: in_base,
        }
    }

    #[test]
    fn klikpajak_example() {
        // Exclusive prices, service 5% inside tax base, tax 10%: 80.000 -> 92.400.
        let t = compute(80_000, 0, rules(1000, 500, false, true), 0);
        assert_eq!((t.service, t.tax, t.total), (4_000, 8_400, 92_400));
    }

    #[test]
    fn service_outside_base() {
        let t = compute(80_000, 0, rules(1000, 500, false, false), 0);
        assert_eq!((t.service, t.tax, t.total), (4_000, 8_000, 92_000));
    }

    #[test]
    fn inclusive_does_not_add_tax() {
        let t = compute(110_000, 0, rules(1000, 0, true, true), 0);
        assert_eq!((t.tax, t.total), (10_000, 110_000));
    }

    #[test]
    fn discount_reduces_the_tax_base() {
        let t = compute(100_000, 10_000, rules(1000, 0, false, true), 0);
        assert_eq!((t.discount, t.tax, t.total), (10_000, 9_000, 99_000));
        // A discount can never exceed the subtotal.
        assert_eq!(compute(5_000, 9_999, rules(1000, 0, false, true), 0).total, 0);
    }

    #[test]
    fn rounding_is_its_own_recorded_line() {
        let t = compute(63_800 - 5_800, 0, rules(1000, 0, false, true), 500);
        assert_eq!((t.total, t.rounding), (64_000, 200));
        assert_eq!(compute(58_000, 0, rules(1000, 0, false, true), 0).rounding, 0);
    }

    #[test]
    fn formats_rupiah() {
        assert_eq!(rupiah(0), "Rp0");
        assert_eq!(rupiah(92_400), "Rp92.400");
        assert_eq!(rupiah(1_234_567), "Rp1.234.567");
    }
}
