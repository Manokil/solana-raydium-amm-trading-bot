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
            init_jito, init_nozomi, init_zslot, BUY_SOL_AMOUNT, CONFIRM_SERVICE, ENTRY_PERCENT, JITO_CLIENT, NOZOMI_CLIENT, POOL_ADDRESS, PRIORITY_FEE, PUBKEY, RPC_CLIENT, SLIPPAGE, ZSLOT_CLIENT
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
    solana_sdk::{commitment_config::CommitmentConfig, instruction::Instruction, pubkey::Pubkey},
    spl_associated_token_account::get_associated_token_address,
    std::{
        collections::{HashMap, HashSet},
        env,
        sync::Arc,
        time::Duration,
    },
    tokio::{
        sync::RwLock,
        time::{interval, sleep},
    },
    yellowstone_grpc_proto::geyser::{CommitmentLevel, SubscribeRequestFilterTransactions},
};

use chrono::Utc;

static POOL_PRICE: Lazy<Arc<RwLock<Option<f64>>>> = Lazy::new(|| Arc::new(RwLock::new(None)));
static LATEST_POOL_PRICE: Lazy<Arc<RwLock<Option<f64>>>> =
    Lazy::new(|| Arc::new(RwLock::new(None)));
static SWAP_BUY_IXS: Lazy<Arc<RwLock<Vec<Instruction>>>> =
    Lazy::new(|| Arc::new(RwLock::new(vec![])));

#[tokio::main]
pub async fn main() -> CarbonResult<()> {
    env_logger::init();
    dotenv::dotenv().ok();

    init_nozomi().await;
    init_zslot().await;
    init_jito().await;

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
    println!("Purchase Amount: {} SOL", *BUY_SOL_AMOUNT as f64 / 10_f64.powf(9.0));
    println!("Pool Address: {}", *POOL_ADDRESS);
    println!("Entry Percent: {} %", *ENTRY_PERCENT);
    println!("Slippage: {} %", *SLIPPAGE * 100.0);

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
        let percent = ((new - old) / old) * 100.0;
        // println!(
        //     "POOL_PRICE changed: old = {:.8}, new = {:.8}, change = {:+.4}%",
        //     old, new, percent
        // );
        if percent <= -*ENTRY_PERCENT {
            println!("ALERT: POOL_PRICE dropped more than {}%!", *ENTRY_PERCENT);
            tokio::spawn(async {
                build_and_submit_swap_transaction().await;
                panic!("ALERT: POOL_PRICE dropped more than {}%!", *ENTRY_PERCENT);
            });
        }
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
            let nozomi = NOZOMI_CLIENT.get().expect("Nozomi client not initialized");

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
                Ok(data) => json!({ "result": data }),
                Err(err) => {
                    json!({ "result": "error", "message": err.to_string() })
                }
            }
        }
        "ZERO_SLOT" => {
            let zero_slot = ZSLOT_CLIENT.get().expect("ZSlot client not initialized");

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
                Ok(data) => json!({ "result": data }),
                Err(err) => {
                    json!({ "result": "error", "message": err.to_string() })
                }
            }
        }
        "JITO" => {
            let jito = JITO_CLIENT.get().expect("Jito client not initialized");

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
                Ok(data) => json!({ "result": data }),
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
        "Transaction confirmed --> : {:#?}\nCurrent time: {:#?}\nPeriod from start: {:?}",
        results,
        Utc::now(),
        start.elapsed()
    );
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

        let static_account_keys = metadata.transaction_metadata.message.static_account_keys();
        let writable_account_keys = &metadata.transaction_metadata.meta.loaded_addresses.writable;
        let readonly_account_keys = &metadata.transaction_metadata.meta.loaded_addresses.readonly;

        let mut account_keys = vec![];

        account_keys.extend(static_account_keys);
        account_keys.extend(writable_account_keys);
        account_keys.extend(readonly_account_keys);

        let instruction_clone: DecodedInstruction<RaydiumAmmV4Instruction> = instruction.clone();

        // let buy_ixs: Vec<Instruction> = match instruction.data {
       let buy_ixs =  match instruction.data {
            RaydiumAmmV4Instruction::SwapBaseIn(_swap_base_in_data) => {
                // Print siganure with timestamp
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

                        let (
                            input_mint,
                            input_reserve,
                            output_mint,
                            output_reserve,
                            pre_input_reserve,
                            pre_output_reserve,
                        ) = if user_coin_ata == arranged.user_source_token_account {
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

                        arranged.user_source_owner = *PUBKEY;
                        arranged.user_source_token_account =
                            get_associated_token_address(&PUBKEY, &input_mint);
                        arranged.user_destination_token_account =
                            get_associated_token_address(&PUBKEY, &output_mint);

                        let post_output_reserve_val = output_reserve
                            .parse::<f64>()
                            .expect("Invalid post_output_reserve");
                        let post_input_reserve_val = input_reserve
                            .parse::<f64>()
                            .expect("Invalid post_input_reserve");
                        let pre_output_reserve_val = pre_output_reserve
                            .parse::<f64>()
                            .expect("Invalid pre_output_reserve");
                        let pre_input_reserve_val = pre_input_reserve
                            .parse::<f64>()
                            .expect("Invalid pre_input_reserve");

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
                                input_change.abs().clone() as f64 / 10f64.powf(9 as f64);

                            let pool_price_sol = (post_input_reserve_val / 10f64.powf(9 as f64))
                                / (post_output_reserve_val / 10f64.powf(mint_decimal as f64));

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
                                output_change.abs().clone() as f64 / 10f64.powf(9 as f64);

                            let pool_price_sol = (post_output_reserve_val / 10f64.powf(9 as f64))
                                / (post_input_reserve_val / 10f64.powf(mint_decimal as f64));

                            {
                                let mut latest = LATEST_POOL_PRICE.write().await;
                                *latest = Some(pool_price_sol);
                            }

                            // println!(
                            //     "Target Sold {} tokens for {} sol, Price with Sol : {} sol",
                            //     base_mint_amount, buy_amount, pool_price_sol
                            // );
                        }

                        let amount_in = if input_mint == WSOL {
                            BUY_SOL_AMOUNT.clone()
                        } else {
                            let token_balance = match RPC_CLIENT
                                .get_token_account_balance_with_commitment(
                                    &arranged.user_source_token_account,
                                    CommitmentConfig::confirmed(),
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

                        let output_reserve_val = output_reserve
                            .parse::<f64>()
                            .expect("Invalid output_reserve");
                        let input_reserve_val =
                            input_reserve.parse::<f64>().expect("Invalid input_reserve");

                        let amount_out =
                            0.997 * (1.0 - *SLIPPAGE) * (amount_in as f64) * output_reserve_val
                                / (input_reserve_val + amount_in as f64);

                        let buy_exact_in_param = SwapBaseIn {
                            amount_in,
                            minimum_amount_out: amount_out as u64,
                        };

                        let mut ix: Vec<Instruction> = vec![];

                        let create_ata_ix =
                            arranged.get_create_idempotent_ata_ix(input_mint, output_mint);

                        ix.extend(create_ata_ix);

                        if input_mint == WSOL {
                            let wsol_ix = arranged.get_wrap_sol(buy_exact_in_param.clone());
                            ix.extend(wsol_ix);
                        };

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

        Ok(())
    }
}
