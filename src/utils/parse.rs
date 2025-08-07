use solana_sdk::{bs58, pubkey::Pubkey};
use solana_transaction_status_client_types::TransactionTokenBalance;
use yellowstone_grpc_proto::prelude::{Message, TransactionStatusMeta};

pub fn get_pre_post_token_balance(
    pre_token_balance: Vec<TransactionTokenBalance>,
    post_token_balance: Vec<TransactionTokenBalance>,
    pool_addr: &str,
    token_mint: &str,
) -> (u64, u64) {
    let extract_amount = |balances: &[TransactionTokenBalance]| {
        balances
            .iter()
            .find(|tb| tb.owner == pool_addr && tb.mint == token_mint)
            .and_then(|tb| Some(tb.ui_token_amount.clone()))
            .and_then(|ui| ui.amount.parse::<u64>().ok())
            .unwrap_or(0)
    };

    let pre_amount = extract_amount(&pre_token_balance);
    let post_amount = extract_amount(&post_token_balance);

    (pre_amount, post_amount)
}

pub fn get_pre_post_sol_balance(
    meta: &TransactionStatusMeta,
    tx_msg: &Message,
    owner: &str,
) -> (u64, u64) {
    let idx = tx_msg
        .account_keys
        .iter()
        .position(|key| bs58::encode(key).into_string() == owner)
        .unwrap_or_default();

    let pre_amount = meta.pre_balances.get(idx).unwrap_or(&0_u64);

    let post_amount = meta.post_balances.get(idx).unwrap_or(&0_u64);

    (pre_amount.clone(), post_amount.clone())
}

pub fn get_coin_pc_mint(
    post_token_balance: &Vec<TransactionTokenBalance>,
    pre_token_balance: &Vec<TransactionTokenBalance>,
    base_vault: Pubkey,
    quote_vault: Pubkey,
    pool_auth: Pubkey,
    account_keys: &[Pubkey],
) -> (Option<(String, String)>, Option<(String, String)>, Option<(String, String)>, Option<(String, String)>) {
    let base_info = post_token_balance
        .iter()
        .find(|tb| {
            tb.owner == pool_auth.to_string()
                && account_keys[tb.account_index as usize] == base_vault
        })
        .map(|ui| (ui.ui_token_amount.amount.clone(), ui.mint.clone()));

    let pre_base_info = pre_token_balance
        .iter()
        .find(|tb| {
            tb.owner == pool_auth.to_string()
                && account_keys[tb.account_index as usize] == base_vault
        })
        .map(|ui| (ui.ui_token_amount.amount.clone(), ui.mint.clone()));

    let quote_info = post_token_balance
        .iter()
        .find(|tb| {
            tb.owner == pool_auth.to_string()
                && account_keys[tb.account_index as usize] == quote_vault
        })
        .map(|ui| (ui.ui_token_amount.amount.clone(), ui.mint.clone()));

    let pre_quote_info = pre_token_balance
        .iter()
        .find(|tb| {
            tb.owner == pool_auth.to_string()
                && account_keys[tb.account_index as usize] == quote_vault
        })
        .map(|ui| (ui.ui_token_amount.amount.clone(), ui.mint.clone()));

    (base_info, quote_info, pre_base_info, pre_quote_info)
}
