use carbon_raydium_amm_v4_decoder::{
    PROGRAM_ID as RAYDIUM_V4_PROGRAM_ID,
    instructions::swap_base_out::{SwapBaseOut, SwapBaseOutInstructionAccounts},
};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use spl_associated_token_account::get_associated_token_address;

use crate::{config::PUBKEY, utils::blockhash::WSOL};

pub trait SwapBaseOutInstructionAccountsExt {
    fn get_swap_base_out_ix(&self, buy_exact_in_param: SwapBaseOut) -> Instruction;
    fn get_create_idempotent_ata_ix(
        &self,
        base_mint: Pubkey,
        quote_mint: Pubkey,
    ) -> Vec<Instruction>;
    fn get_create_ata_ix(&self) -> Instruction;
    fn get_close_wsol(&self) -> Instruction;
}

impl SwapBaseOutInstructionAccountsExt for SwapBaseOutInstructionAccounts {
    fn get_create_ata_ix(&self) -> Instruction {
        let create_ata_ix =
            spl_associated_token_account::instruction::create_associated_token_account(
                &self.user_source_owner,
                &self.user_source_owner,
                &self.user_source_owner,
                &self.token_program,
            );

        create_ata_ix
    }

    fn get_create_idempotent_ata_ix(
        &self,
        base_mint: Pubkey,
        quote_mint: Pubkey,
    ) -> Vec<Instruction> {
        let create_ata_base_ix =
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &self.user_source_owner,
                &self.user_source_owner,
                &base_mint,
                &self.token_program,
            );

        let create_ata_quote_ix =
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &self.user_source_owner,
                &self.user_source_owner,
                &quote_mint,
                &self.token_program,
            );

        vec![create_ata_base_ix, create_ata_quote_ix]
    }

    fn get_close_wsol(&self) -> Instruction {
        let wsol_ata = get_associated_token_address(&PUBKEY, &WSOL);

        let create_ata_ix =
            spl_token::instruction::close_account(&spl_token::ID, &wsol_ata, &PUBKEY, &PUBKEY, &[])
                .unwrap();

        create_ata_ix
    }

    fn get_swap_base_out_ix(&self, buy_exact_in_param: SwapBaseOut) -> Instruction {
        let discriminator = [9];
        let mut data = Vec::new();

        data.extend_from_slice(&discriminator);
        data.extend_from_slice(&buy_exact_in_param.max_amount_in.to_le_bytes());
        data.extend_from_slice(&buy_exact_in_param.amount_out.to_le_bytes());

        let accounts = if let Some(amm_target_orders) = self.amm_target_orders {
            vec![
                AccountMeta::new_readonly(self.token_program, false),
                AccountMeta::new(self.amm, false),
                AccountMeta::new_readonly(self.amm_authority, false),
                AccountMeta::new(self.amm_open_orders, false),
                AccountMeta::new(amm_target_orders, false),
                AccountMeta::new(self.pool_coin_token_account, false),
                AccountMeta::new(self.pool_pc_token_account, false),
                AccountMeta::new_readonly(self.serum_program, false),
                AccountMeta::new(self.serum_market, false),
                AccountMeta::new(self.serum_bids, false),
                AccountMeta::new(self.serum_asks, false),
                AccountMeta::new(self.serum_event_queue, false),
                AccountMeta::new(self.serum_coin_vault_account, false),
                AccountMeta::new(self.serum_pc_vault_account, false),
                AccountMeta::new_readonly(self.serum_vault_signer, false),
                AccountMeta::new(self.user_source_token_account, false),
                AccountMeta::new(self.user_destination_token_account, false),
                AccountMeta::new(self.user_source_owner, true),
            ]
        } else {
            vec![
                AccountMeta::new_readonly(self.token_program, false),
                AccountMeta::new(self.amm, false),
                AccountMeta::new_readonly(self.amm_authority, false),
                AccountMeta::new(self.amm_open_orders, false),
                AccountMeta::new(self.pool_coin_token_account, false),
                AccountMeta::new(self.pool_pc_token_account, false),
                AccountMeta::new_readonly(self.serum_program, false),
                AccountMeta::new(self.serum_market, false),
                AccountMeta::new(self.serum_bids, false),
                AccountMeta::new(self.serum_asks, false),
                AccountMeta::new(self.serum_event_queue, false),
                AccountMeta::new(self.serum_coin_vault_account, false),
                AccountMeta::new(self.serum_pc_vault_account, false),
                AccountMeta::new_readonly(self.serum_vault_signer, false),
                AccountMeta::new(self.user_source_token_account, false),
                AccountMeta::new(self.user_destination_token_account, false),
                AccountMeta::new(self.user_source_owner, true),
            ]
        };

        Instruction {
            program_id: RAYDIUM_V4_PROGRAM_ID,
            accounts,
            data,
        }
    }
}
