use {
    async_trait::async_trait,
    carbon_core::{
        deserialize::ArrangeAccounts,
        error::CarbonResult,
        instruction::{
            DecodedInstruction, InstructionProcessorInputType,
        },
        metrics::MetricsCollection,
        processor::Processor,
    },
    carbon_log_metrics::LogMetrics,
    carbon_raydium_amm_v4_decoder::{
        PROGRAM_ID as RAY_V4_PROGRAM_ID, RaydiumAmmV4Decoder,
        instructions::{RaydiumAmmV4Instruction, swap_base_in::SwapBaseIn},
    },
    carbon_yellowstone_grpc_datasource::YellowstoneGrpcGeyserClient,
    once_cell::sync::Lazy,
    raydium_amm_monitor::{
        config::{
            BUY_SOL_AMOUNT, CONFIRM_SERVICE, JITO_CLIENT, NOZOMI_CLIENT, PRIORITY_FEE, PUBKEY,
            RPC_CLIENT, SLIPPAGE, POOL_ADDRESS, ZSLOT_CLIENT, init_jito, init_nozomi, init_zslot,
        },
        instructions::SwapBaseInInstructionAccountsExt,
        service::Tips,
        utils::{
            blockhash::{WSOL, get_slot, recent_blockhash_handler},
            build_and_sign::build_and_sign,
            parse::{get_coin_pc_mint},
        },
    },
    serde_json::json,
    solana_sdk::{
        commitment_config::CommitmentConfig, instruction::Instruction, pubkey::Pubkey,
    },
    spl_associated_token_account::get_associated_token_address,
    std::{
        collections::{HashMap, HashSet},
        env,
        sync::Arc,
        time::{Duration},
    },
    tokio::{sync::RwLock, time::sleep},
    yellowstone_grpc_proto::geyser::{CommitmentLevel, SubscribeRequestFilterTransactions},
};

use chrono::Utc;

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

    println!("Using payer: {}", *PUBKEY);

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

pub static SIGNATURES: Lazy<RwLock<HashSet<String>>> = Lazy::new(|| RwLock::new(HashSet::new()));

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
        let start = std::time::Instant::now();

        let raw_instruction: Vec<Instruction> = match instruction.data {
            RaydiumAmmV4Instruction::SwapBaseIn(swap_base_in_data) => {
                // Check if signature already processed
                if SIGNATURES.read().await.contains(&signature.to_string()) {
                    return Ok(());
                } else {
                    SIGNATURES.write().await.insert(signature.to_string());
                }
                // Spawn a task to remove the signature after 3 seconds
                tokio::spawn(async move {
                    sleep(Duration::from_secs(3)).await;
                    SIGNATURES.write().await.remove(&signature.to_string());
                });

                // Print siganure with timestamp
                println!(
                    "Received target's signature : {:#?}\nCurrent time : {:#?}",
                    signature,
                    Utc::now()
                );

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
                        .post_token_balances
                        .clone();

                    let pre_token_balances_for_chain = pre_token_balances.clone();

                    let full_token_balances: Vec<_> = post_token_balance
                        .clone()
                        .into_iter()
                        .chain(pre_token_balances_for_chain.into_iter())
                        .collect();

                    let (coin_raw_info, pc_raw_info, pre_coin_raw_info, pre_pc_raw_info) = get_coin_pc_mint(
                        post_token_balance.as_ref().unwrap_or(&vec![]),
                        pre_token_balances.as_ref().unwrap_or(&vec![]),
                        arranged.pool_coin_token_account,
                        arranged.pool_pc_token_account,
                        arranged.amm_authority,
                        &account_keys,
                    );

                    if let (Some(coin_info), Some(pc_info), Some(pre_coin_info), Some(pre_pc_info)) = (coin_raw_info, pc_raw_info, pre_coin_raw_info, pre_pc_raw_info) {
                        let user_coin_ata = get_associated_token_address(
                            &arranged.user_source_owner,
                            &Pubkey::from_str_const(&coin_info.1),
                        );

                        let (input_mint, input_reserve, output_mint, output_reserve, pre_input_reserve, pre_output_reserve) =
                            if user_coin_ata == arranged.user_source_token_account {
                                (coin_info.1, coin_info.0, pc_info.1, pc_info.0, pre_coin_info.0, pre_pc_info.0)
                            } else {
                                (pc_info.1, pc_info.0, coin_info.1, coin_info.0, pre_pc_info.0, pre_coin_info.0)
                            };

                        let input_mint = Pubkey::from_str_const(&input_mint);
                        let output_mint = Pubkey::from_str_const(&output_mint);

                        println!("input_mint: {}, output_mint: {}", input_mint, output_mint);

                        // Get balance of base mint
                        let mut mint_decimal: u8 = 6;

                        arranged.user_source_owner = *PUBKEY;
                        arranged.user_source_token_account =
                            get_associated_token_address(&PUBKEY, &input_mint);
                        arranged.user_destination_token_account =
                            get_associated_token_address(&PUBKEY, &output_mint);

                        let amount_in = if input_mint == WSOL {
                            mint_decimal = full_token_balances
                                .iter()
                                .flat_map(|balances| balances.iter())
                                .find(|balance| balance.mint == output_mint.to_string())
                                .and_then(|balance| Some(balance.ui_token_amount.decimals))
                                .unwrap_or(6);
                            BUY_SOL_AMOUNT.clone()
                        } else {
                            mint_decimal = full_token_balances
                                .iter()
                                .flat_map(|balances| balances.iter())
                                .find(|balance| balance.mint == input_mint.to_string())
                                .and_then(|balance| Some(balance.ui_token_amount.decimals))
                                .unwrap_or(6);
                            let token_balance = match RPC_CLIENT
                                .get_token_account_balance_with_commitment(
                                    &arranged.user_source_token_account,
                                    CommitmentConfig::confirmed(),
                                )
                                .await
                            {
                                Ok(response) => response.value.amount,
                                Err(e) => {
                                    eprintln!("Failed to get token balance: {:?}", e);
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
                            // Calculate token amount with decimals
                            let base_mint_amount =
                                output_change.abs() as f64 / 10f64.powf(mint_decimal as f64);
                            let buy_amount =
                                input_change.abs().clone() as f64 / 10f64.powf(9 as f64);

                            println!(
                                "Target Bought {} tokens with {} sol",
                                base_mint_amount, buy_amount
                            );
                            // println!(
                            //     "input_change: {}, output_change: {}, Amount In: {}, Amount Out: {}",
                            //     input_change, output_change, amount_in, amount_out
                            // );
                        } else {
                            // Calculate token amount with decimals
                            let base_mint_amount =
                                input_change.abs() as f64 / 10f64.powf(mint_decimal as f64);
                            let buy_amount =
                                output_change.abs().clone() as f64 / 10f64.powf(9 as f64);
                            println!(
                                "Target Sold {} tokens for {} sol",
                                base_mint_amount, buy_amount
                            );
                        }

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

                        ix
                    } else {
                        vec![]
                    }
                } else {
                    println!("Failed to arrange accounts");

                    vec![]
                }
            }
            _ => {
                vec![]
            }
        };

        if !raw_instruction.is_empty() {
            let (cu, priority_fee_micro_lamport, third_party_fee) = *PRIORITY_FEE;

            // Print current timestamp and consumed time from start
            println!(
                "Submitting tx --> Current time: {:#?}\nPeriod from start: {:?}",
                Utc::now(),
                start.elapsed()
            );

            let results = match CONFIRM_SERVICE.as_str() {
                "NOZOMI" => {
                    let nozomi = NOZOMI_CLIENT.get().expect("Nozomi client not initialized");

                    let ixs = nozomi.add_tip_ix(Tips {
                        cu: Some(cu),
                        priority_fee_micro_lamport: Some(priority_fee_micro_lamport),
                        payer: *PUBKEY,
                        pure_ix: raw_instruction.clone(),
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
                        pure_ix: raw_instruction,
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
                        pure_ix: raw_instruction,
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
        };

        Ok(())
    }
}
