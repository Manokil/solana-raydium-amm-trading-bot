use {
    async_trait::async_trait,
    carbon_core::{
        deserialize::ArrangeAccounts,
        error::CarbonResult,
        instruction::{DecodedInstruction, InstructionProcessorInputType},
        metrics::MetricsCollection,
        processor::Processor,
    },
    carbon_log_metrics::LogMetrics,
    carbon_raydium_amm_v4_decoder::{
        instructions::{
            swap_base_in::SwapBaseIn, swap_base_out::SwapBaseOut, RaydiumAmmV4Instruction
        }, RaydiumAmmV4Decoder, PROGRAM_ID as RAY_V4_PROGRAM_ID
    },
    carbon_yellowstone_grpc_datasource::YellowstoneGrpcGeyserClient,
    once_cell::sync::Lazy,
    raydium_amm_monitor::{
        config::{
            init_jito, init_nozomi, init_zslot, AUTO_EXIT, BUY_SOL_AMOUNT, CONFIRM_SERVICE, ENTRY_PERCENT, ENTRY_SLIPPAGE, EXIT_SLIPPAGE, JITO_CLIENT, NOZOMI_CLIENT, POOL_ADDRESS, PRIORITY_FEE, PUBKEY, RPC_CLIENT, STOP_LOSS, TAKE_PROFIT, ZSLOT_CLIENT
        },
        instructions::{SwapBaseInInstructionAccountsExt, SwapBaseOutInstructionAccountsExt},
        service::Tips,
        utils::{
            blockhash::{get_slot, recent_blockhash_handler, WSOL},
            build_and_sign::build_and_sign,
            parse::get_coin_pc_mint,
        },
    },
    serde_json::json,
    serde_json::to_value,
    solana_sdk::{commitment_config::CommitmentConfig, exit, instruction::Instruction, pubkey::Pubkey},
    spl_associated_token_account::get_associated_token_address,
    std::{
        collections::{HashMap, HashSet}, env, process, sync::Arc, time::Duration
    },
    tokio::{
        sync::RwLock,
        time::{interval, sleep},
    },
    yellowstone_grpc_proto::geyser::{CommitmentLevel, SubscribeRequestFilterTransactions},
};

use chrono::Utc;
use mongodb::{bson::doc, Client, options::ClientOptions};
use libc::_exit;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Trade_Data {
    ts: String,
    profit_sol: f64,
    fees_lamports: i64,
    fees_sol: f64,
    roi_pct: f64,
    program_runtime_ms: i64,
}



static POOL_PRICE: Lazy<Arc<RwLock<Option<f64>>>> = Lazy::new(|| Arc::new(RwLock::new(None)));
static LATEST_POOL_PRICE: Lazy<Arc<RwLock<Option<f64>>>> =
    Lazy::new(|| Arc::new(RwLock::new(None)));
static SWAP_BUY_IXS: Lazy<Arc<RwLock<Vec<Instruction>>>> =
    Lazy::new(|| Arc::new(RwLock::new(vec![])));
static IS_BOUGHT: Lazy<Arc<RwLock<bool>>> = Lazy::new(|| Arc::new(RwLock::new(false)));
static BOUGHT_PRICE: Lazy<Arc<RwLock<Option<f64>>>> = Lazy::new(|| Arc::new(RwLock::new(None)));
static BOUGHT_AT_TIME: Lazy<Arc<RwLock<Option<i64>>>> = Lazy::new(|| Arc::new(RwLock::new(None)));
static INITIAL_WSOL_BALANCE: Lazy<Arc<RwLock<Option<f64>>>> = Lazy::new(|| Arc::new(RwLock::new(None)));
static SIGNATURE: Lazy<Arc<RwLock<Option<String>>>> = Lazy::new(|| Arc::new(RwLock::new(None)));
static START_TIME: Lazy<Arc<RwLock<Option<std::time::Instant>>>> =
    Lazy::new(|| Arc::new(RwLock::new(None)));
static LAST_PROFIT_SOL: Lazy<Arc<RwLock<Option<f64>>>> =
    Lazy::new(|| Arc::new(RwLock::new(None)));
static LAST_INPUT_LAMPORTS_DELTA: Lazy<Arc<RwLock<Option<i128>>>> =
    Lazy::new(|| Arc::new(RwLock::new(None)));
static LAST_OUTPUT_LAMPORTS_DELTA: Lazy<Arc<RwLock<Option<i128>>>> =
    Lazy::new(|| Arc::new(RwLock::new(None)));
static LAST_ROI_PCT: Lazy<Arc<RwLock<Option<f64>>>> =
    Lazy::new(|| Arc::new(RwLock::new(None)));

// Track total fees from transactions
static FEES: Lazy<Arc<RwLock<f64>>> = Lazy::new(|| Arc::new(RwLock::new(0.0)));

// Store the last calculated duration
static LAST_DURATION: Lazy<Arc<RwLock<Option<std::time::Duration>>>> = 
    Lazy::new(|| Arc::new(RwLock::new(None)));

#[tokio::main]
pub async fn main() -> CarbonResult<()> {
    env_logger::init();
    dotenv::dotenv().ok();

    init_nozomi().await;
    init_zslot().await;
    init_jito().await;

    {
        let mut start_time_guard = START_TIME.write().await;
        *start_time_guard = Some(std::time::Instant::now());
        println!("Start time: {:?}", *start_time_guard);
    }
    tokio::spawn({
        async move {
            loop {
                recent_blockhash_handler(RPC_CLIENT.clone()).await;
            }
        }
    });

    // NOTE: Workaround, that solving issue https://github.com/rustls/rustls/issues/1877
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Can't set crypto provider to aws_lc_rs");

    let transaction_filter = SubscribeRequestFilterTransactions {
        vote: Some(false),
        failed: Some(false),
        account_include: vec![POOL_ADDRESS.to_string()],
        account_exclude: vec![],
        account_required: vec![RAY_V4_PROGRAM_ID.to_string().clone()],
        signature: None,
    };

    println!("My Wallet: {}", *PUBKEY);
    println!(
        "Purchase Amount: {} SOL",
        *BUY_SOL_AMOUNT as f64 / 10_f64.powf(9.0)
    );
    println!("Pool Address: {}", *POOL_ADDRESS);
    println!("Entry Percent: {}%", *ENTRY_PERCENT);
    println!("Entry Slippage: {}%", *ENTRY_SLIPPAGE);
    println!("Stop Loss: {}%", *STOP_LOSS);
    println!("Take Profit: {}%", *TAKE_PROFIT);
    println!("Auto Exit: {} seconds", *AUTO_EXIT);

    // Spawn a task to store pool_price_sol every 400ms

    tokio::spawn({
        let latest_price = LATEST_POOL_PRICE.clone();
        let pool_price = POOL_PRICE.clone();

        async move {
            let mut ticker = interval(Duration::from_millis(400));
            loop {
                ticker.tick().await;
                let mut latest = pool_price.write().await;
                let latest_val = *latest_price.read().await;
                if let (Some(new), Some(old)) = (latest_val, *latest) {
                    display_pool_price_change(old, new);
                }
                *latest = latest_val;
            }
        }
    });

    let mut transaction_filters: HashMap<String, SubscribeRequestFilterTransactions> =
        HashMap::new();

    transaction_filters.insert(
        "jupiter_swap_transaction_filter".to_string(),
        transaction_filter,
    );

    let yellowstone_grpc = YellowstoneGrpcGeyserClient::new(
        env::var("GEYSER_URL").unwrap_or_default(),
        env::var("X_TOKEN").ok(),
        Some(CommitmentLevel::Processed),
        HashMap::new(),
        transaction_filters.clone(),
        Default::default(),
        Arc::new(RwLock::new(HashSet::new())),
    );

    println!("Starting RAYDIUM V4 Monitor...");

    carbon_core::pipeline::Pipeline::builder()
        .datasource(yellowstone_grpc)
        .metrics(Arc::new(LogMetrics::new()))
        .metrics_flush_interval(3)
        .instruction(RaydiumAmmV4Decoder, RaydiumV4Process)
        .shutdown_strategy(carbon_core::pipeline::ShutdownStrategy::Immediate)
        .build()?
        .run()
        .await?;

    println!("Raydium Launchpad Monitor has stopped.");

    Ok(())
}

fn display_pool_price_change(old: f64, new: f64) {
    if old > 0.0 {
        tokio::spawn({
            let bought = IS_BOUGHT.clone();

            async move {
                let bought = *bought.read().await;

                if !bought {
                    let new_clone = new.clone();
                    // Fix division by zero bug - ensure old price is not zero
                    let percent = if old > 0.0 {
                        ((new_clone - old) / old) * 100.0
                    } else {
                        0.0 // Default to 0% if old price is zero or negative
                    };
                    println!(
                        "POOL_PRICE changed: old = {:.8}, new = {:.8}, change = {:+.4}%",
                        old, new_clone, percent
                    );
                    if percent <= -*ENTRY_PERCENT {
                        println!("ALERT: POOL_PRICE dropped more than {}%!", *ENTRY_PERCENT);
                        tokio::spawn(async move {
                            let new_price_clone = new_clone.clone();              

                            let mut bought_price = BOUGHT_PRICE.write().await;
                            *bought_price = Some(new_price_clone);

                            let mut bought_at = BOUGHT_AT_TIME.write().await;
                            let current_time = Utc::now().timestamp_millis();
                            *bought_at = Some(current_time);

                            build_and_submit_swap_transaction().await;
                        });
                    }
                } else {
                    let new_clone = new.clone();
                    let bought_price = BOUGHT_PRICE.write().await;
                    let old_bought_price = bought_price.clone();
                    // Fix division by zero bug - ensure old_bought_price is not zero
                    let percent = if let Some(old_price) = old_bought_price {
                        if old_price > 0.0 {
                            ((new_clone - old_price) / old_price) * 100.0
                        } else {
                            0.0 // Default to 0% if old price is zero or negative
                        }
                    } else {
                        0.0 // Default to 0% if no bought price available
                    };
                    if let Some(old_price) = old_bought_price {
                        println!("POOL_PRICE changed : buy price = {:.8}, current price = {:.8}, change = {:.8}", old_price, new_clone, percent);
                    }
                    let bought_at = BOUGHT_AT_TIME.write().await;
                    let current_time = Utc::now().timestamp_millis();

                    // Check how long it flowed from bought_at
                    if percent >= *TAKE_PROFIT {
                        println!("ALERT: POOL_PRICE increased more than {}%!", percent);                                          
                        // Reset IS_BOUGHT to false when selling
                        build_and_submit_swap_transaction().await;
                        sleep(Duration::from_secs(3)).await;
                     
                         // Persist metrics to MongoDB (best-effort)
                         let profit_sol = LAST_PROFIT_SOL.read().await.unwrap_or(0.0);
                         let total_fees = *FEES.read().await;
                         let roi_pct = LAST_ROI_PCT.read().await.unwrap_or(0.0);
                         let duration_ms = LAST_DURATION.read().await
                             .as_ref()
                             .map(|d| d.as_millis() as i64)
                             .unwrap_or(0);                         
                         let _ = save_trade_metrics(profit_sol, total_fees, roi_pct, duration_ms).await;
                         
                         process::exit(0);
                       
                    } else if percent <= -*STOP_LOSS {
                        println!("ALERT: POOL_PRICE decreased more than {}%!", percent);                                         
                        
                        // Reset IS_BOUGHT to false when selling
                        build_and_submit_swap_transaction().await;   
                        sleep(Duration::from_secs(3)).await;
                        // Persist metrics to MongoDB (best-effort)
                        let profit_sol = LAST_PROFIT_SOL.read().await.unwrap_or(0.0);
                        let total_fees = *FEES.read().await;
                        let roi_pct = LAST_ROI_PCT.read().await.unwrap_or(0.0);
                        let duration_ms = LAST_DURATION.read().await
                            .as_ref()
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0);
                        let _ = save_trade_metrics(profit_sol, total_fees, roi_pct, duration_ms).await;
                        
                        process::exit(0);

                    } else if let Some(bought_at_time) = *bought_at {
                        // Fix unsafe unwrap() by using safer conversion
                        let auto_exit_ms = match (*AUTO_EXIT * 1000).try_into() {
                            Ok(ms) => ms,
                            Err(_) => {
                                println!("Warning: AUTO_EXIT conversion failed, using default 1000ms");
                                1000i64
                            }
                        };
                        if current_time - bought_at_time > auto_exit_ms {
                            println!("ALERT: AUTO EXIT triggered after {} seconds!", *AUTO_EXIT);                         
                            process::exit(0);
                        }
                    }
                }
            }
        });
    }
}

async fn build_and_submit_swap_transaction() {
    let (cu, priority_fee_micro_lamport, third_party_fee) = *PRIORITY_FEE;

    let start = std::time::Instant::now();

    // Print current timestamp and consumed time from start
    println!("Submitting tx --> Current time: {:#?}", Utc::now(),);

    let buy_ixs = {
        let buy_ixs_guard = SWAP_BUY_IXS.read().await;
        buy_ixs_guard.clone()
    };

    if buy_ixs.is_empty() {
        println!("No swap instructions to submit.");
        return;
    }

    let results = match CONFIRM_SERVICE.as_str() {
        "NOZOMI" => {
            let nozomi = match NOZOMI_CLIENT.get() {
                Some(client) => client,
                None => {
                    println!("Error: Nozomi client not initialized");
                    return;
                }
            };

            let ixs = nozomi.add_tip_ix(Tips {
                cu: Some(cu),
                priority_fee_micro_lamport: Some(priority_fee_micro_lamport),
                payer: *PUBKEY,
                pure_ix: buy_ixs.clone(),
                tip_addr_idx: 1,
                tip_sol_amount: third_party_fee,
            });

            let recent_blockhash = get_slot();

            let encoded_tx = build_and_sign(ixs, recent_blockhash, None);

            match nozomi.send_transaction(&encoded_tx).await {
                Ok(data) => {
                    // Extract signature from the result
                    *SIGNATURE.write().await = Some(data["result"].as_str().unwrap_or_default().to_string());
                    json!({ "result": data })
                },
                Err(err) => {
                    json!({ "result": "error", "message": err.to_string() })
                }
            }
        }
        "ZERO_SLOT" => {
            let zero_slot = match ZSLOT_CLIENT.get() {
                Some(client) => client,
                None => {
                    println!("Error: ZSlot client not initialized");
                    return;
                }
            };

            let ixs = zero_slot.add_tip_ix(Tips {
                cu: Some(cu),
                priority_fee_micro_lamport: Some(priority_fee_micro_lamport),
                payer: *PUBKEY,
                pure_ix: buy_ixs,
                tip_addr_idx: 1,
                tip_sol_amount: third_party_fee,
            });

            let recent_blockhash = get_slot();

            let encoded_tx = build_and_sign(ixs, recent_blockhash, None);

            match zero_slot.send_transaction(&encoded_tx).await {
                Ok(data) => {
                    // Extract signature from the result
                    *SIGNATURE.write().await = Some(data["result"].as_str().unwrap_or_default().to_string());
                    json!({ "result": data })
                },
                Err(err) => {
                    json!({ "result": "error", "message": err.to_string() })
                }
            }
        }
        "JITO" => {
            let jito = match JITO_CLIENT.get() {
                Some(client) => client,
                None => {
                    println!("Error: Jito client not initialized");
                    return;
                }
            };

            let ixs = jito.add_tip_ix(Tips {
                cu: Some(cu),
                priority_fee_micro_lamport: Some(priority_fee_micro_lamport),
                payer: *PUBKEY,
                pure_ix: buy_ixs,
                tip_addr_idx: 1,
                tip_sol_amount: third_party_fee,
            });

            let recent_blockhash = get_slot();

            let encoded_tx = build_and_sign(ixs, recent_blockhash, None);

            match jito.send_transaction(&encoded_tx).await {
                Ok(data) => {
                    // Extract signature from the result
                    *SIGNATURE.write().await = Some(data["result"].as_str().unwrap_or_default().to_string());
                    json!({ "result": data })
                },
                Err(err) => {
                    json!({ "result": "error", "message": err.to_string() })
                }
            }
        }
        _ => {
            json!({ "result": "error", "message": "unknown confirmation service" })
        }
    };

    println!(
        "Transaction submitting --> : {:#?}\nCurrent time: {:#?}\nPeriod from start: {:?}",
        results,
        Utc::now(),
        start.elapsed()
    );
}

async fn save_trade_metrics(
    profit_sol: f64,
    total_fees_lamports_f64: f64,
    roi_pct: f64,
    duration_ms: i64,
) -> mongodb::error::Result<()> {
    println!("Saving trade metrics");
    let uri = std::env::var("MONGODB_URI").expect("MONGODB_URI not set");
    let options = ClientOptions::parse(uri).await?;
    let client = Client::with_options(options)?;

    // 2. Get database and collection
    let db = client.database("mydb");
    let collection = db.collection::<Trade_Data>("trade_data");

    // Convert the BSON document to the Trade_Data struct for correct type insertion
    let new_data = Trade_Data {
        ts: Utc::now().to_rfc3339(),
        profit_sol: profit_sol,
        fees_lamports: total_fees_lamports_f64 as i64,
        fees_sol: total_fees_lamports_f64 / 1_000_000_000.0,
        roi_pct: roi_pct,
        program_runtime_ms: duration_ms,
    };

    let doc_debug = format!("{:#?}", new_data);
    collection.insert_one(new_data).await?;
    println!("Saved trade metrics: {}", doc_debug);
    Ok(())
}


pub struct RaydiumV4Process;

#[async_trait]
impl Processor for RaydiumV4Process {
    type InputType = InstructionProcessorInputType<RaydiumAmmV4Instruction>;

    async fn process(
        &mut self,
        (metadata, instruction, _nested_instructions, _instructions): Self::InputType,
        _metrics: Arc<MetricsCollection>,
    ) -> CarbonResult<()> {
        let signature = metadata.transaction_metadata.signature;      
        
        //println!("metadata: {:#?}", metadata);
        //println!("instruction: {:#?}", instruction);
        //println!("nested_instructions: {:?}", _nested_instructions);
        //println!("instructions: {:#?}", _instructions);
       
        let static_account_keys = metadata.transaction_metadata.message.static_account_keys();
        let writable_account_keys = &metadata.transaction_metadata.meta.loaded_addresses.writable;
        let readonly_account_keys = &metadata.transaction_metadata.meta.loaded_addresses.readonly;

        let mut account_keys = vec![];

        account_keys.extend(static_account_keys);
        account_keys.extend(writable_account_keys);
        account_keys.extend(readonly_account_keys);
        //println!("account_keys: {:#?}", account_keys);

        let instruction_clone: DecodedInstruction<RaydiumAmmV4Instruction> = instruction.clone();

        // let buy_ixs: Vec<Instruction> = match instruction.data {
        let _buy_ixs = match instruction.data {
            RaydiumAmmV4Instruction::SwapBaseIn(_swap_base_in_data) => {
                // Print signature with timestamp
                // println!(
                //     "Received target's signature : {:#?}\nCurrent time : {:#?}",
                //     signature,
                //     Utc::now()
                // );

                if let Some(mut arranged) =
                    SwapBaseIn::arrange_accounts(&instruction_clone.accounts)
                {
                    let post_token_balance = metadata
                        .transaction_metadata
                        .meta
                        .post_token_balances
                        .clone();

                    let pre_token_balances = metadata
                        .transaction_metadata
                        .meta
                        .pre_token_balances
                        .clone();

                    let pre_token_balances_for_chain = pre_token_balances.clone();

                    let full_token_balances: Vec<_> = post_token_balance
                        .clone()
                        .into_iter()
                        .chain(pre_token_balances_for_chain.into_iter())
                        .collect();

                    let (coin_raw_info, pc_raw_info, pre_coin_raw_info, pre_pc_raw_info) =
                        get_coin_pc_mint(
                            post_token_balance.as_ref().unwrap_or(&vec![]),
                            pre_token_balances.as_ref().unwrap_or(&vec![]),
                            arranged.pool_coin_token_account,
                            arranged.pool_pc_token_account,
                            arranged.amm_authority,
                            &account_keys,
                        );

                    if let (
                        Some(coin_info),
                        Some(pc_info),
                        Some(pre_coin_info),
                        Some(pre_pc_info),
                    ) = (
                        coin_raw_info,
                        pc_raw_info,
                        pre_coin_raw_info,
                        pre_pc_raw_info,
                    ) {
                        let user_coin_ata = get_associated_token_address(
                            &arranged.user_source_owner,
                            &Pubkey::from_str_const(&coin_info.1),
                        );
                        let user_coin1_ata = get_associated_token_address(
                            &arranged.user_source_owner,
                            &Pubkey::from_str_const(&pc_info.1),
                        );

                        let (
                            input_mint,
                            input_reserve,
                            output_mint,
                            output_reserve,
                            pre_input_reserve,
                            pre_output_reserve,
                        ) = if (user_coin_ata == arranged.user_source_token_account)
                            || (user_coin1_ata == arranged.user_destination_token_account)
                        {
                            (
                                coin_info.1,
                                coin_info.0,
                                pc_info.1,
                                pc_info.0,
                                pre_coin_info.0,
                                pre_pc_info.0,
                            )
                        } else {
                            (
                                pc_info.1,
                                pc_info.0,
                                coin_info.1,
                                coin_info.0,
                                pre_pc_info.0,
                                pre_coin_info.0,
                            )
                        };

                        let input_mint = Pubkey::from_str_const(&input_mint);
                        let output_mint = Pubkey::from_str_const(&output_mint);

                        // Get balance of base mint
                        let mut mint_decimal: u8 = 6;

                        let post_output_reserve_val = match output_reserve.parse::<f64>() {
                            Ok(val) => val,
                            Err(_) => {
                                println!("Warning: Invalid output_reserve value: {}", output_reserve);
                                return Ok(());
                            }
                        };
                        let post_input_reserve_val = match input_reserve.parse::<f64>() {
                            Ok(val) => val,
                            Err(_) => {
                                println!("Warning: Invalid input_reserve value: {}", input_reserve);
                                return Ok(());
                            }
                        };
                        let pre_output_reserve_val = match pre_output_reserve.parse::<f64>() {
                            Ok(val) => val,
                            Err(_) => {
                                println!("Warning: Invalid pre_output_reserve value: {}", pre_output_reserve);
                                return Ok(());
                            }
                        };
                        let pre_input_reserve_val = match pre_input_reserve.parse::<f64>() {
                            Ok(val) => val,
                            Err(_) => {
                                println!("Warning: Invalid pre_input_reserve value: {}", pre_input_reserve);
                                return Ok(());
                            }
                        };

                        let input_change = post_input_reserve_val - pre_input_reserve_val;
                        let output_change = post_output_reserve_val - pre_output_reserve_val;

                        if input_mint == WSOL {
                            mint_decimal = full_token_balances
                                .iter()
                                .flat_map(|balances| balances.iter())
                                .find(|balance| balance.mint == output_mint.to_string())
                                .and_then(|balance| Some(balance.ui_token_amount.decimals))
                                .unwrap_or(6);
                            // Calculate token amount with decimals
                            let base_mint_amount =
                                output_change.abs() as f64 / 10f64.powf(mint_decimal as f64);
                            let buy_amount =
                                input_change.abs() as f64 / 10f64.powf(9 as f64);

                            let pool_price_sol = if post_output_reserve_val > 0.0 {
                                (post_input_reserve_val / 10f64.powf(9 as f64))
                                    / (post_output_reserve_val / 10f64.powf(mint_decimal as f64))
                            } else {
                                0.0 // Default to 0 if output reserve is zero
                            };

                            {
                                let mut latest = LATEST_POOL_PRICE.write().await;
                                *latest = Some(pool_price_sol);
                            }

                            // println!(
                            //     "Target Bought {} tokens with {} sol, Price with Sol : {} sol",
                            //     base_mint_amount, buy_amount, pool_price_sol
                            // );
                        } else {
                            mint_decimal = full_token_balances
                                .iter()
                                .flat_map(|balances| balances.iter())
                                .find(|balance| balance.mint == input_mint.to_string())
                                .and_then(|balance| Some(balance.ui_token_amount.decimals))
                                .unwrap_or(6);
                            // Calculate token amount with decimals
                            let base_mint_amount =
                                input_change.abs() as f64 / 10f64.powf(mint_decimal as f64);
                            let buy_amount =
                                output_change.abs() as f64 / 10f64.powf(9 as f64);

                            let pool_price_sol = if post_input_reserve_val > 0.0 {
                                (post_output_reserve_val / 10f64.powf(9 as f64))
                                    / (post_input_reserve_val / 10f64.powf(mint_decimal as f64))
                            } else {
                                0.0 // Default to 0 if input reserve is zero
                            };

                            {
                                let mut latest = LATEST_POOL_PRICE.write().await;
                                *latest = Some(pool_price_sol);
                            }

                            // println!(
                            //     "Target Sold {} tokens for {} sol, Price with Sol : {} sol",
                            //     base_mint_amount, buy_amount, pool_price_sol
                            // );
                        }

                        arranged.user_source_owner = *PUBKEY;
                        arranged.user_source_token_account =
                            get_associated_token_address(&PUBKEY, &input_mint);
                        arranged.user_destination_token_account =
                            get_associated_token_address(&PUBKEY, &output_mint);

                        let has_bought = *IS_BOUGHT.read().await;

                        if !has_bought {
                            if input_mint == WSOL {
                                arranged.user_source_token_account =
                                    get_associated_token_address(&PUBKEY, &input_mint);
                                arranged.user_destination_token_account =
                                    get_associated_token_address(&PUBKEY, &output_mint);
                            } else {
                                arranged.user_source_token_account =
                                    get_associated_token_address(&PUBKEY, &output_mint);
                                arranged.user_destination_token_account =
                                    get_associated_token_address(&PUBKEY, &input_mint);
                            }
                        } else {
                            if input_mint == WSOL {
                                arranged.user_source_token_account =
                                    get_associated_token_address(&PUBKEY, &output_mint);
                                arranged.user_destination_token_account =
                                    get_associated_token_address(&PUBKEY, &input_mint);
                            } else {
                                arranged.user_source_token_account =
                                    get_associated_token_address(&PUBKEY, &input_mint);
                                arranged.user_destination_token_account =
                                    get_associated_token_address(&PUBKEY, &output_mint);
                            }
                        }

                        let amount_in = if !has_bought {
                            BUY_SOL_AMOUNT.clone()
                        } else {
                            let token_balance = match RPC_CLIENT
                                .get_token_account_balance_with_commitment(
                                    &arranged.user_source_token_account,
                                    CommitmentConfig::processed(),
                                )
                                .await
                            {
                                Ok(response) => response.value.amount,
                                Err(e) => {
                                    return Ok(());
                                }
                            };

                            let token_amount = match token_balance.parse::<u64>() {
                                Ok(amount) => amount,
                                Err(e) => {
                                    return Ok(());
                                }
                            };

                            token_amount
                        };

                        let output_reserve_val = match output_reserve.parse::<f64>() {
                            Ok(val) => val,
                            Err(_) => {
                                println!("Warning: Invalid output_reserve value: {}", output_reserve);
                                return Ok(());
                            }
                        };
                        let input_reserve_val = match input_reserve.parse::<f64>() {
                            Ok(val) => val,
                            Err(_) => {
                                println!("Warning: Invalid input_reserve value: {}", input_reserve);
                                return Ok(());
                            }
                        };

                        // Calculate amount_out by entry_slippage/exit slippage when buying
                        // Add safety check to prevent division by zero
                        let amount_out = if input_reserve_val + amount_in as f64 > 0.0 {
                            if has_bought {
                                0.997
                                    * (1.0 - *EXIT_SLIPPAGE)
                                    * (amount_in as f64)
                                    * output_reserve_val
                                    / (input_reserve_val + amount_in as f64)
                            } else {
                                0.997
                                    * (1.0 - *ENTRY_SLIPPAGE)
                                    * (amount_in as f64)
                                    * output_reserve_val
                                    / (input_reserve_val + amount_in as f64)
                            }
                        } else {
                            println!("Warning: Invalid reserve values for amount_out calculation");
                            return Ok(());
                        };

                        let buy_exact_in_param = SwapBaseIn {
                            amount_in,
                            minimum_amount_out: amount_out as u64,
                        };

                        let mut ix: Vec<Instruction> = vec![];

                        let create_ata_ix =
                            arranged.get_create_idempotent_ata_ix(input_mint, output_mint);

                        ix.extend(create_ata_ix);

                        // if input_mint == WSOL {
                        //     let wsol_ix = arranged.get_wrap_sol(buy_exact_in_param.clone());
                        //     ix.extend(wsol_ix);
                        // };

                        let swap_ix = arranged.get_swap_base_in_ix(buy_exact_in_param.clone());
                        ix.push(swap_ix);

                        {
                            let mut buy_ixs = SWAP_BUY_IXS.write().await;
                            *buy_ixs = ix;
                        }
                    }
                }
            }

            _ => {
                println!("");
            }
        };

        let sent_signature = SIGNATURE.read().await.clone();
        let metadata_signature = metadata.transaction_metadata.signature.to_string();
        let metadata_fee = metadata.transaction_metadata.meta.fee;
        let wsol_ata = get_associated_token_address(&PUBKEY, &WSOL);
        let Some(idx) = account_keys.iter().position(|key| key == &wsol_ata) else {           
            return Ok(());
        };
       // let wsol_lamports = metadata.transaction_metadata.meta.pre_balances[idx] - metadata.transaction_metadata.meta.post_balances[idx];
        if let Some(sig) = sent_signature {
            if sig == metadata_signature {
                let mut has_bought = IS_BOUGHT.write().await;
                *has_bought = !*has_bought;
                println!("Transaction signature confirmed: {}", sig);
                println!("IS_BOUGHT STATE: {}", *has_bought);
                println!("Transaction fee: {}", metadata_fee);
                let mut fees = FEES.write().await;
                *fees += metadata_fee as f64;
                println!("Total fees: {}", *fees);
               // println!("metadata: {:#?}", metadata);
                // Compute SOL deltas using signed math and convert lamports -> SOL
                let pre_lamports = metadata.transaction_metadata.meta.pre_balances.get(idx).copied().unwrap_or(0) as i128;
                let post_lamports = metadata.transaction_metadata.meta.post_balances.get(idx).copied().unwrap_or(0) as i128;

                let mut input_lamports_delta: i128 = 0;   // lamports spent (buy)
                let mut output_lamports_delta: i128 = 0;  // lamports received (sell)

                if *has_bought {
                    // Just bought: SOL decreased
                    input_lamports_delta = pre_lamports - post_lamports;
                    let mut last_in = LAST_INPUT_LAMPORTS_DELTA.write().await;
                    *last_in = Some(input_lamports_delta);
                    let input_sol = input_lamports_delta as f64 / 1_000_000_000.0;
                    println!("Input SOL: {}", input_sol);
                } else {
                    // Just sold: SOL increased
                    output_lamports_delta = post_lamports - pre_lamports;
                    let mut last_out = LAST_OUTPUT_LAMPORTS_DELTA.write().await;
                    *last_out = Some(output_lamports_delta);
                    let output_sol = output_lamports_delta as f64 / 1_000_000_000.0;
                    println!("Output SOL: {}", output_sol);
                    let profit_sol = (output_lamports_delta - LAST_INPUT_LAMPORTS_DELTA.read().await.unwrap_or(0)) as f64 / 1_000_000_000.0;
                    println!("Profit: {}", profit_sol);
                    {
                        let mut last_profit = LAST_PROFIT_SOL.write().await;
                        *last_profit = Some(profit_sol);
                    }
                    println!("Total fees: {}", *fees);
                    let last_input_lamports = *LAST_INPUT_LAMPORTS_DELTA.read().await;
                    let input_sol = last_input_lamports.unwrap_or(0) as f64 / 1_000_000_000.0;
                    let roi = if input_sol > 0.0 { (profit_sol / input_sol) * 100.0 } else { 0.0 };
                    println!("ROI: {}", roi);
                    {
                        let mut last_roi = LAST_ROI_PCT.write().await;
                        *last_roi = Some(roi);
                    }
                }

                if !*has_bought {
                    if let Some(start_time) = START_TIME.read().await.as_ref() {
                        let end_time = std::time::Instant::now();
                        println!("End time: {:?}", end_time);
                        let duration = end_time.duration_since(*start_time);
                        println!("Time taken: {:?}", duration);

                        // Save duration to static variable
                        {
                            let mut last_duration = LAST_DURATION.write().await;
                            *last_duration = Some(duration);
                        }

                       
                    } else {
                        println!("No start time found");    
                    }
                }
            }    
        }

        Ok(())
    }
}
