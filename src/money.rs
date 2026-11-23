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
    pub service: i64,
    pub tax: i64,
    pub total: i64,
}

/// Round half up on non-negative integers.
fn mul_bp(amount: i64, bp: i64) -> i64 {
    (amount * bp + 5_000) / 10_000
}

pub fn compute(subtotal: i64, r: TaxRules) -> Totals {
    let service = mul_bp(subtotal, r.service_bp);
    if r.tax_inclusive {
        // Prices already contain tax: extract it, never add it.
        let gross = if r.service_in_tax_base {
            subtotal + service
        } else {
            subtotal
        };
        let tax = gross - (gross * 10_000 + (10_000 + r.tax_bp) / 2) / (10_000 + r.tax_bp);
        Totals {
            subtotal,
            service,
            tax,
            total: subtotal + service,
        }
    } else {
        let base = if r.service_in_tax_base {
            subtotal + service
        } else {
            subtotal
        };
        let tax = mul_bp(base, r.tax_bp);
        Totals {
            subtotal,
            service,
            tax,
            total: subtotal + service + tax,
        }
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

    #[test]
    fn klikpajak_example() {
        // Exclusive prices, service 5% inside tax base, tax 10%: 80.000 -> 92.400.
        let t = compute(
            80_000,
            TaxRules {
                tax_bp: 1000,
                service_bp: 500,
                tax_inclusive: false,
                service_in_tax_base: true,
            },
        );
        assert_eq!((t.service, t.tax, t.total), (4_000, 8_400, 92_400));
    }

    #[test]
    fn service_outside_base() {
        let t = compute(
            80_000,
            TaxRules {
                tax_bp: 1000,
                service_bp: 500,
                tax_inclusive: false,
                service_in_tax_base: false,
            },
        );
        assert_eq!((t.service, t.tax, t.total), (4_000, 8_000, 92_000));
    }

    #[test]
    fn inclusive_does_not_add_tax() {
        let t = compute(
            110_000,
            TaxRules {
                tax_bp: 1000,
                service_bp: 0,
                tax_inclusive: true,
                service_in_tax_base: true,
            },
        );
        assert_eq!((t.tax, t.total), (10_000, 110_000));
    }

    #[test]
    fn formats_rupiah() {
        assert_eq!(rupiah(0), "Rp0");
        assert_eq!(rupiah(92_400), "Rp92.400");
        assert_eq!(rupiah(1_234_567), "Rp1.234.567");
    }
}
