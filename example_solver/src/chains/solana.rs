pub mod solana_chain {
    use crate::chains::*;
    use crate::routers::jupiter::create_token_account;
    use crate::routers::jupiter::jupiter_swap;
    use crate::routers::jupiter::quote;
    use crate::routers::jupiter::Memo as Jup_Memo;
    use crate::routers::jupiter::QuoteConfig;
    use crate::routers::jupiter::SwapMode;
    use crate::PostIntentInfo;
    use num_bigint::BigInt;
    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_sdk::commitment_config::CommitmentConfig;
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::signature::{Keypair, Signer};
    use solana_sdk::transaction::Transaction;
    use spl_associated_token_account::get_associated_token_address;
    use spl_token::instruction::transfer;
    use std::env;
    use std::str::FromStr;

    #[derive(Debug, Serialize, Deserialize)]
    struct SwapData {
        pub user_account: String,
        pub token_in: String,
        pub token_out: String,
        pub amount: u64,
        pub slippage_bps: u64,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct Memo {
        tx_hash: String,
        intent_id: String,
        params: Vec<String>,
    }

    pub async fn solana_executing(
        intent_id: &str,
        intent: PostIntentInfo,
        _amount: &str,
    ) -> String {
        let mut msg = String::default();
        let rpc_url = env::var("SOLANA_RPC").expect("SOLANA_RPC must be set");
        let from_keypair = Keypair::from_base58_string(
            env::var("SOLANA_KEYPAIR")
                .expect("SOLANA_KEYPAIR must be set")
                .as_str(),
        );
        let client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());

        match intent.function_name.as_str() {
            "transfer" => {
                let mut user_account = String::default();
                let mut token_out = String::default();
                let mut amount = 0u64;

                if let OperationOutput::SwapTransfer(transfer_output) = &intent.outputs {
                    user_account = transfer_output.dst_chain_user.clone();
                    token_out = transfer_output.token_out.clone();
                    amount = transfer_output.amount_out.parse::<u64>().unwrap();
                }

                msg = match transfer_slp20(
                    &client,
                    &from_keypair,
                    &Pubkey::from_str(&user_account).unwrap(),
                    &Pubkey::from_str(&token_out).unwrap(),
                    amount,
                )
                .await
                {
                    Ok(signature) => json!({
                        "code": 1,
                        "msg": {
                            "intent_id": intent_id,
                            "solver_id": SOLVER_ID.to_string(),
                            "signature": signature,
                        }
                    })
                    .to_string(),
                    Err(err) => json!({
                        "code": 0,
                        "solver_id": 0,
                        "msg": format!("Transaction failed: {}", err)
                    })
                    .to_string(),
                }
            }
            "swap" => {
                let mut user_account = String::default();
                let mut token_in = String::default();
                let mut token_out = String::default();
                let mut amount = 0u64;

                if let OperationOutput::SwapTransfer(transfer_output) = &intent.outputs {
                    user_account = transfer_output.dst_chain_user.clone();
                    token_out = transfer_output.token_out.clone();
                    amount = transfer_output.amount_out.parse::<u64>().unwrap();
                }
                if let OperationInput::SwapTransfer(transfer_input) = &intent.inputs {
                    token_in = transfer_input.token_in.clone();
                }

                let memo = format!(
                    r#"{{"user_account": "{}","token_in": "{}","token_out": "{}","amount_in": {},"slippage_bps": {}}}"#,
                    user_account, token_in, token_out, amount, 100
                );

                let signature = jupiter_swap(&memo, &client, &from_keypair).await.unwrap();
                msg = json!({
                    "code": 1,
                    "msg": {
                        "intent_id": intent_id,
                        "solver_id": SOLVER_ID.to_string(),
                        "signature": signature.to_string(),
                    }
                })
                .to_string();
            }
            _ => {
                println!("function not supported")
            }
        }

        msg
    }

    async fn transfer_slp20(
        client: &RpcClient,
        sender_keypair: &Keypair,
        recipient_wallet_pubkey: &Pubkey,
        token_mint_pubkey: &Pubkey,
        amount: u64,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let sender_wallet_pubkey = &sender_keypair.pubkey();
        let sender_token_account_pubkey =
            get_associated_token_address(sender_wallet_pubkey, token_mint_pubkey);
        let recipient_token_account_pubkey =
            get_associated_token_address(recipient_wallet_pubkey, token_mint_pubkey);

        if client
            .get_account(&sender_token_account_pubkey)
            .await
            .is_err()
        {
            eprintln!("Sender's associated token account does not exist");
            return Err("Sender's associated token account does not exist".into());
        }

        if client
            .get_account(&recipient_token_account_pubkey)
            .await
            .is_err()
        {
            create_token_account(
                recipient_wallet_pubkey,
                token_mint_pubkey,
                sender_keypair,
                client,
            )
            .await
            .unwrap();
        }

        let recent_blockhash = client.get_latest_blockhash().await.unwrap();
        let transfer_instruction = transfer(
            &spl_token::id(),
            &sender_token_account_pubkey,
            &recipient_token_account_pubkey,
            &sender_keypair.pubkey(),
            &[],
            amount,
        )
        .unwrap();

        let transaction = Transaction::new_signed_with_payer(
            &[transfer_instruction],
            Some(&sender_keypair.pubkey()),
            &[sender_keypair],
            recent_blockhash,
        );

        let simulation_result = client.simulate_transaction(&transaction).await.unwrap();
        if simulation_result.value.err.is_some() {
            eprintln!(
                "Transaction simulation failed: {:?}",
                simulation_result.value.err
            );
            return Err("Transaction simulation failed".into());
        }

        let result = client
            .send_and_confirm_transaction_with_spinner(&transaction)
            .await?;

        Ok(result.to_string())
    }

    pub async fn _get_solana_token_decimals(
        token_address: &str,
    ) -> Result<u8, Box<dyn std::error::Error>> {
        let rpc_url = env::var("SOLANA_RPC").expect("SOLANA_RPC must be set");
        let client = reqwest::Client::new();
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTokenSupply",
            "params": [
                token_address
            ]
        });

        let response = client
            .post(rpc_url)
            .json(&request_body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        if let Some(decimals) = response["result"]["value"]["decimals"].as_u64() {
            Ok(decimals as u8)
        } else {
            Err("Token information not available.".into())
        }
    }

    pub async fn solana_simulate_swap(
        dst_chain_user: &str,
        bridge_token_address_dst: &str,
        token_out: &str,
        amount_in_dst_chain: u64,
    ) -> String {
        let memo = format!(
            r#"{{
                "user_account": "{}",
                "token_in": "{}",
                "token_out": "{}",
                "amount": {},
                "slippage_bps": {}
            }}"#,
            dst_chain_user, bridge_token_address_dst, token_out, amount_in_dst_chain, 100
        );

        let memo = Jup_Memo::from_json(&memo).unwrap();

        let only_direct_routes = false;
        // get jupiter quote
        let quotes = quote(
            memo.token_in,
            memo.token_out,
            memo.amount,
            QuoteConfig {
                only_direct_routes,
                swap_mode: Some(SwapMode::ExactIn),
                slippage_bps: Some(memo.slippage_bps),
                ..QuoteConfig::default()
            },
        )
        .await
        .unwrap();

        BigInt::from(quotes.out_amount).to_string()
    }

    pub async fn solana_send_funds_to_user(
        token_mint: &str,
        amount: &str,
        solver_out: &str,
        single_domain: bool,
    ) {
        // call send_funds_to_user on https://github.com/ComposableFi/emulated-light-client/blob/fast-bridge/solana/bridge-escrow/programs/bridge-escrow/src/lib.rs
    }
}
