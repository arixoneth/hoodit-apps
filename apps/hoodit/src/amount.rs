use num_bigint::BigUint;
use num_traits::Zero;
use std::str::FromStr;

pub fn atomic(value: &str) -> Result<BigUint, String> {
    if value.is_empty()
        || value.len() > 78
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err("invalid canonical uint256 amount".into());
    }
    let value =
        BigUint::from_str(value).map_err(|_| "invalid unsigned atomic amount".to_string())?;
    if value.bits() > 256 {
        return Err("atomic amount exceeds uint256".into());
    }
    Ok(value)
}
/// Exact conversion of a whole-unit decimal string into atomic units. More
/// fractional digits than the token supports is an error, never a rounding.
pub fn from_decimal(value: &str, decimals: u8) -> Result<BigUint, String> {
    let value = crate::model::decimal(value)?;
    let (whole, fractional) = value.split_once('.').unwrap_or((&value, ""));
    if fractional.len() > decimals as usize {
        return Err(format!(
            "amount has more than {decimals} fractional digits for this token"
        ));
    }
    let digits = format!("{whole}{fractional:0<width$}", width = decimals as usize);
    let digits = digits.trim_start_matches('0');
    atomic(if digits.is_empty() { "0" } else { digits })
}
pub fn fraction(value: &BigUint, bps: u16) -> BigUint {
    value * BigUint::from(bps) / BigUint::from(10_000u16)
}
pub fn format(value: &BigUint, decimals: u8) -> String {
    if decimals == 0 {
        return value.to_string();
    }
    let scale = BigUint::from(10u8).pow(decimals as u32);
    let whole = value / &scale;
    let rem = value % &scale;
    if rem.is_zero() {
        return whole.to_string();
    }
    let frac = format!("{:0>width$}", rem, width = decimals as usize)
        .trim_end_matches('0')
        .to_string();
    format!("{whole}.{frac}")
}
#[cfg(test)]
pub fn max_uint256() -> BigUint {
    use num_traits::One;
    (BigUint::one() << 256usize) - BigUint::one()
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_traits::One;
    #[test]
    fn exact_amounts() {
        assert_eq!(atomic(&max_uint256().to_string()).unwrap(), max_uint256());
        assert!(atomic(&(BigUint::one() << 256usize).to_string()).is_err());
        assert_eq!(format(&atomic("1234500").unwrap(), 6), "1.2345");
        assert_eq!(fraction(&atomic("999").unwrap(), 5000).to_string(), "499");
        let max = max_uint256();
        assert_eq!(fraction(&max, 10_000), max);
    }
    #[test]
    fn decimal_amounts_convert_exactly() {
        assert_eq!(
            from_decimal("0.05", 18).unwrap().to_string(),
            "50000000000000000"
        );
        assert_eq!(
            from_decimal("2000000", 18).unwrap().to_string(),
            "2000000000000000000000000"
        );
        assert_eq!(from_decimal("1.5", 6).unwrap().to_string(), "1500000");
        assert_eq!(from_decimal("0", 6).unwrap().to_string(), "0");
        assert!(from_decimal("0.1234567", 6).is_err());
        assert!(from_decimal("-1", 6).is_err());
        assert!(from_decimal("1e5", 6).is_err());
    }
}
