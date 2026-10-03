//! Exact token amounts. A decimal the token can't represent is an error,
//! never a rounding.
use num_bigint::BigUint;
use num_traits::Zero;
use std::str::FromStr;

fn atomic(digits: &str) -> Result<BigUint, String> {
    let value = BigUint::from_str(digits).map_err(|_| "invalid amount".to_string())?;
    if value.bits() > 256 {
        return Err("amount exceeds uint256".into());
    }
    Ok(value)
}

/// Whole-unit decimal string ("0.05") to atomic units.
pub fn from_decimal(value: &str, decimals: u8) -> Result<BigUint, String> {
    let value = value.trim();
    let (whole, fractional) = value.split_once('.').unwrap_or((value, ""));
    let plain = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty()
        || value.len() > 96
        || !plain(whole)
        || !plain(fractional)
        || (value.contains('.') && fractional.is_empty())
    {
        return Err("expected a plain non-negative decimal such as 0.05".into());
    }
    if fractional.len() > decimals as usize {
        return Err(format!("this token supports at most {decimals} decimals"));
    }
    let digits = format!("{whole}{fractional:0<width$}", width = decimals as usize);
    let digits = digits.trim_start_matches('0');
    atomic(if digits.is_empty() { "0" } else { digits })
}

/// Atomic units back to a whole-unit decimal string.
pub fn format(value: &BigUint, decimals: u8) -> String {
    if decimals == 0 {
        return value.to_string();
    }
    let scale = BigUint::from(10u8).pow(decimals as u32);
    let (whole, rem) = (value / &scale, value % &scale);
    if rem.is_zero() {
        return whole.to_string();
    }
    let frac = format!("{:0>width$}", rem, width = decimals as usize);
    format!("{whole}.{}", frac.trim_end_matches('0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_convert_exactly() {
        assert_eq!(
            from_decimal("0.05", 18).unwrap().to_string(),
            "50000000000000000"
        );
        assert_eq!(from_decimal("1.5", 6).unwrap().to_string(), "1500000");
        assert_eq!(from_decimal("0", 6).unwrap().to_string(), "0");
        assert!(from_decimal("0.1234567", 6).is_err());
        assert!(from_decimal("-1", 6).is_err());
        assert!(from_decimal("1e5", 6).is_err());
        assert!(from_decimal("1.", 6).is_err());
        assert_eq!(format(&from_decimal("1.2345", 6).unwrap(), 6), "1.2345");
    }
}
