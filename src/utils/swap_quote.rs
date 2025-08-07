use std::ops::{Add, Div, Mul};

use crate::utils::utils::{FEE_RATE, TRADE_FEE_RATE, calculate_fee};

pub fn get_amount_out(amount_in: u128, input_reserve: u128, output_reserve: u128) -> u128 {
    if input_reserve + amount_in == 0 {
        return 0;
    }

    let numerator = amount_in.mul(output_reserve);
    let denominator = input_reserve.add(amount_in);

    numerator.div(denominator)
}

pub fn get_swap_quote(amount_in: u64, base_reserve: u64, quote_reserve: u64) -> u64 {
    let fee = calculate_fee(amount_in, TRADE_FEE_RATE + FEE_RATE);

    let amount_less_fee = amount_in - fee;

    let result = get_amount_out(
        amount_less_fee as u128,
        quote_reserve as u128,
        base_reserve as u128,
    );

    result as u64
}

#[cfg(test)]
mod tests {
    use crate::utils::utils::{FEE_RATE, TRADE_FEE_RATE, calculate_fee};

    use super::*;

    // https://solscan.io/tx/4XFZ9UGWAfHkiRquibHokJ8ANP46p9vUrr1zwsWpQXazxK7fn8VhkGNcEWLG2i2vk5jCBbJuMT3CoPoBuj96wstu#tokenBalanceChange

    #[test]
    fn test_swap_base_input_without_fees() {
        let amount_in = 693000000_u64;
        let base_reserve = 1073025605596382_u64 - 555337575467276_u64;
        let quote_reserve = 30000852951_u64 + 32182704639_u64;

        let fee = calculate_fee(amount_in, TRADE_FEE_RATE + FEE_RATE);

        let amount_less_fee = amount_in - fee;

        println!("Without Fee : {}", amount_less_fee);

        let result = get_amount_out(
            amount_less_fee as u128,
            quote_reserve as u128,
            base_reserve as u128,
        );

        println!("{}", result);

        let result1 = get_swap_quote(amount_in, base_reserve, quote_reserve);
        
        println!("{}", result1);
    }
}
