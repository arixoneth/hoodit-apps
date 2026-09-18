use num_bigint::BigUint;
use num_integer::Integer;
use num_traits::{One, Zero};
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
pub fn extrapolate(
    balance: &BigUint,
    input: &BigUint,
    output: &BigUint,
    input_decimals: u8,
    output_decimals: u8,
) -> Option<(String, String)> {
    if input.is_zero() {
        return None;
    }
    let ten = BigUint::from(10u8);
    let unit_num = output * ten.pow(input_decimals as u32);
    let unit_den = input * ten.pow(output_decimals as u32);
    let value_num = output * balance;
    let value_den = input * ten.pow(output_decimals as u32);
    Some((
        ratio_decimal(&unit_num, &unit_den, 36),
        ratio_decimal(&value_num, &value_den, 36),
    ))
}
fn ratio_decimal(num: &BigUint, den: &BigUint, places: u32) -> String {
    let scale = BigUint::from(10u8).pow(places);
    let (mut rounded, remainder) = (num * &scale).div_rem(den);
    let twice = &remainder << 1usize;
    if twice > *den || (twice == *den && (&rounded & BigUint::one()) == BigUint::one()) {
        rounded += BigUint::one();
    }
    format(&rounded, places as u8)
}
#[cfg(test)]
pub fn max_uint256() -> BigUint {
    (BigUint::one() << 256usize) - BigUint::one()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_amounts() {
        assert_eq!(atomic(&max_uint256().to_string()).unwrap(), max_uint256());
        assert!(atomic(&(BigUint::one() << 256usize).to_string()).is_err());
        assert_eq!(format(&atomic("1234500").unwrap(), 6), "1.2345");
        assert_eq!(fraction(&atomic("999").unwrap(), 5000).to_string(), "499");
    }
    #[test]
    fn quote_math() {
        let (unit, value) = extrapolate(
            &atomic("10000000000000000000000").unwrap(),
            &atomic("100000000000000000000").unwrap(),
            &atomic("1000000").unwrap(),
            18,
            6,
        )
        .unwrap();
        assert_eq!((unit.as_str(), value.as_str()), ("0.01", "100"));
    }
    #[test]
    fn rounds_rational_half_even_and_values_independently() {
        assert_eq!(
            ratio_decimal(&BigUint::from(1u8), &BigUint::from(8u8), 2),
            "0.12"
        );
        assert_eq!(
            ratio_decimal(&BigUint::from(3u8), &BigUint::from(8u8), 2),
            "0.38"
        );
        let max = max_uint256();
        assert_eq!(fraction(&max, 10_000), max);
    }
}
