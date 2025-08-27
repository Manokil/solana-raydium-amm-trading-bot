pub const TRADE_FEE_RATE: u64 = 2500_u64;
pub const FEE_RATE: u64 = 10000_u64;

const FEE_RATE_DENOMINATOR_VALUE: u64 = 1_000_000_u64;

pub fn ceil_div(token_amount: u64, fee_numerator: u64, fee_denominator: u64) -> u64 {
    if fee_denominator == 0 {
        panic!("Division by zero");
    }

    (token_amount
        .saturating_mul(fee_numerator)
        .saturating_add(fee_denominator - 1))
        / fee_denominator
}

pub fn calculate_fee(amount: u64, fee_rate: u64) -> u64 {
    ceil_div(amount, fee_rate, FEE_RATE_DENOMINATOR_VALUE)
}
