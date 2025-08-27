use dotenvy::dotenv;
use once_cell::sync::Lazy;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub static CONFIRM_SERVICE: Lazy<String> =
    Lazy::new(|| env::var("CONFIRM_SERVICE").expect("CONFIRM_SERVICE must be set"));

pub static PRIORITY_FEE: Lazy<(u64, u64, f64)> = Lazy::new(|| {
    dotenv().ok();

    let cu = env::var("CU")
        .ok()
        .and_then(|val| val.parse::<u64>().ok())
        .unwrap_or(0); // fallback if missing or invalid

    let priority_fee_micro_lamport = env::var("PRIORITY_FEE_MICRO_LAMPORT")
        .ok()
        .and_then(|val| val.parse::<u64>().ok())
        .unwrap_or(0); // fallback if missing or invalid

    let third_party_fee = env::var("THIRD_PARTY_FEE")
        .ok()
        .and_then(|val| val.parse::<f64>().ok())
        .unwrap_or(0.0); // fallback if missing or invalid

    (cu, priority_fee_micro_lamport, third_party_fee)
});

pub static BUY_SOL_AMOUNT: Lazy<u64> = Lazy::new(|| {
    dotenv().ok(); // load .env if available

    let val = env::var("BUY_SOL_AMOUNT").expect("Missing env var: BUY_SOL_AMOUNT");

    let buy_sol_amount = val.parse::<f64>().unwrap_or_else(|e| {
        eprintln!("Invalid BUY_SOL_AMOUNT '{}': {}", val, e);
        std::process::exit(1);
    });

    (buy_sol_amount * 10_f64.powf(9.0)) as u64
});

pub static ENTRY_SLIPPAGE: Lazy<f64> = Lazy::new(|| {
    dotenv().ok(); // load .env if available

    let raw = env::var("ENTRY_SLIPPAGE").unwrap_or_else(|_| "1.0".to_string()); // default to "1.0%"
    let parsed: f64 = raw.parse().expect("Failed to parse ENTRY_SLIPPAGE");
    parsed / 100.0 // convert percent to decimal (e.g., 1.0 -> 0.01)
});

pub static EXIT_SLIPPAGE: Lazy<f64> = Lazy::new(|| {
    dotenv().ok(); // load .env if available

    let raw = env::var("EXIT_SLIPPAGE").unwrap_or_else(|_| "1.0".to_string()); // default to "1.0%"
    let parsed: f64 = raw.parse().expect("Failed to parse EXIT_SLIPPAGE");
    parsed / 100.0 // convert percent to decimal (e.g., 1.0 -> 0.01)
});

pub static ENTRY_PERCENT: Lazy<f64> = Lazy::new(|| {
    dotenv().ok(); // load .env if available

    let raw = env::var("ENTRY_PERCENT").unwrap_or_else(|_| "100.0".to_string()); // default to "100.0%"
    let parsed: f64 = raw.parse().expect("Failed to parse ENTRY_PERCENT");
    parsed // convert percent to decimal (e.g., 1.0 -> 0.01)
});

pub static TAKE_PROFIT: Lazy<f64> = Lazy::new(|| {
    dotenv().ok(); // load .env if available

    let raw = env::var("TAKE_PROFIT").unwrap_or_else(|_| "100.0".to_string()); // default to "100.0%"
    let parsed: f64 = raw.parse().expect("Failed to parse TAKE_PROFIT");
    parsed // convert percent to decimal (e.g., 1.0 -> 0.01)
});

pub static STOP_LOSS: Lazy<f64> = Lazy::new(|| {
    dotenv().ok(); // load .env if available

    let raw = env::var("STOP_LOSS").unwrap_or_else(|_| "100.0".to_string()); // default to "100.0%"
    let parsed: f64 = raw.parse().expect("Failed to parse STOP_LOSS");
    parsed // convert percent to decimal (e.g., 1.0 -> 0.01)
});

pub static AUTO_EXIT: Lazy<u64> = Lazy::new(|| {
    dotenv().ok(); // load .env if available

    let raw = env::var("AUTO_EXIT").unwrap_or_else(|_| "60".to_string()); // default to "60 seconds"
    raw.parse::<u64>().expect("Failed to parse AUTO_EXIT")
});
