mod chains;
mod routers;

use crate::chains::ethereum::ethereum_chain::ethereum_executing;
use crate::chains::ethereum::ethereum_chain::ethereum_send_funds_to_user;
use crate::chains::ethereum::ethereum_chain::get_evm_token_decimals;
use crate::chains::ethereum::ethereum_chain::send_tx;
use crate::chains::ethereum::ethereum_chain::UsdtContract;
use crate::chains::ethereum::ethereum_chain::ERC20;
use crate::chains::ethereum::ethereum_chain::ESCROW_SC_ETHEREUM;
use crate::chains::get_token_info;
use crate::chains::solana::solana_chain::solana_executing;
use crate::chains::OperationInput;
use crate::chains::OperationOutput;
use crate::chains::PostIntentInfo;
use crate::chains::INTENTS;
use crate::chains::SOLVER_ADDRESSES;
use crate::chains::SOLVER_ID;
use crate::chains::SOLVER_PRIVATE_KEY;
use crate::routers::get_simulate_swap_intent;
use crate::routers::jupiter::jupiter_swap;
use crate::routers::paraswap::paraswap_router::simulate_swap_paraswap;
use crate::routers::paraswap::paraswap_router::ParaswapParams;
use chains::create_keccak256_signature;
use chains::ethereum::ethereum_chain::approve_erc20;
use chains::ethereum::ethereum_chain::PARASWAP;
use chains::solana::solana_chain::solana_send_funds_to_user;
use ethers::providers::{Http, Provider};
use ethers::types::Address;
use ethers::types::U256;
use futures::{SinkExt, StreamExt};
use num_bigint::BigInt;
use reqwest::Client;
use routers::jupiter::SwapMode;
use serde_json::json;
use serde_json::Value;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signer::keypair::Keypair;
use solana_sdk::signer::Signer;
use spl_associated_token_account::get_associated_token_address;
use std::env;
use std::str::FromStr;
use std::sync::Arc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    let server_addr = env::var("COMPOSABLE_ENDPOINT").unwrap_or_else(|_| String::from(""));

    let (ws_stream, _) = connect_async(server_addr).await.expect("Failed to connect");
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    let mut json_data = json!({
        "code": 1,
        "msg": {
            "solver_id": SOLVER_ID.to_string(),
            "solver_addresses": SOLVER_ADDRESSES,
        }
    });

    create_keccak256_signature(&mut json_data, SOLVER_PRIVATE_KEY.to_string())
        .await
        .unwrap();

    if json_data.get("code").unwrap() == "0" {
        println!("{:#?}", json_data);
        return;
    }

    ws_sender
        .send(Message::Text(json_data.to_string()))
        .await
        .expect("Failed to send initial message");

    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                let code = parsed.get("code").unwrap().as_u64().unwrap();

                println!("{:#?}", parsed);

                if code == 0 {
                    // error
                } else if code == 1 {
                    // participate auction
                    let intent_id = parsed
                        .get("msg")
                        .unwrap()
                        .get("intent_id")
                        .and_then(Value::as_str)
                        .unwrap();
                    let intent_str = parsed
                        .get("msg")
                        .unwrap()
                        .get("intent")
                        .unwrap()
                        .to_string();
                    let intent_value: Value = serde_json::from_str(&intent_str).unwrap();
                    let intent_info: PostIntentInfo = serde_json::from_value(intent_value).unwrap();

                    // calculate best quote
                    let final_amount = get_simulate_swap_intent(
                        &intent_info,
                        &intent_info.src_chain,
                        &intent_info.dst_chain,
                        &String::from("USDT"),
                    )
                    .await;

                    // decide if participate or not
                    let mut amount_out_min = U256::zero();
                    if let OperationOutput::SwapTransfer(transfer_output) = &intent_info.outputs {
                        amount_out_min = U256::from_dec_str(&transfer_output.amount_out).unwrap();
                    }

                    let final_amount = U256::from_dec_str(&final_amount).unwrap();

                    println!("User wants {amount_out_min} token_out, you can provide {final_amount} token_out (after FLAT_FEES + COMISSION)");

                    if final_amount > amount_out_min {
                        let mut json_data = json!({
                            "code": 2,
                            "msg": {
                                "intent_id": intent_id,
                                "solver_id": SOLVER_ID.to_string(),
                                "amount": final_amount.to_string()
                            }
                        });

                        create_keccak256_signature(&mut json_data, SOLVER_PRIVATE_KEY.to_string())
                            .await
                            .unwrap();

                        ws_sender
                            .send(Message::text(json_data.to_string()))
                            .await
                            .expect("Failed to send message");

                        let mut intents = INTENTS.write().await;
                        intents.insert(intent_id.to_string(), intent_info);
                        drop(intents);
                    }
                } else if code == 2 {
                    // TO DO
                    // check if tx from rollup went ok
                } else if code == 3 {
                    // solver registered
                } else if code == 4 {
                    // auction result
                    let intent_id = parsed
                        .get("msg")
                        .unwrap()
                        .get("intent_id")
                        .and_then(Value::as_str)
                        .unwrap();
                    let amount = parsed
                        .get("msg")
                        .unwrap()
                        .get("amount")
                        .and_then(Value::as_str)
                        .unwrap();
                    let msg = parsed
                        .get("msg")
                        .unwrap()
                        .get("msg")
                        .and_then(Value::as_str)
                        .unwrap()
                        .to_string();

                    if msg.contains("won") {
                        let intent;
                        {
                            let intents = INTENTS.read().await;
                            intent = intents.get(intent_id).unwrap().clone();
                            drop(intents);
                        }

                        if intent.dst_chain == "solana" {
                            let from_keypair = Keypair::from_base58_string(
                                env::var("SOLANA_KEYPAIR")
                                    .expect("SOLANA_KEYPAIR must be set")
                                    .as_str(),
                            );
                            let rpc_url = env::var("SOLANA_RPC").expect("SOLANA_RPC must be set");
                            let client = RpcClient::new_with_commitment(
                                rpc_url,
                                CommitmentConfig::confirmed(),
                            );

                            let usdt_contract_address =
                                "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";

                            let mut user_account = String::default();
                            let mut token_in = String::default();
                            let mut token_out = String::default();
                            let mut amount_in = String::default();

                            let usdt_token_account = get_associated_token_address(
                                &from_keypair.pubkey(),
                                &Pubkey::from_str(usdt_contract_address).unwrap(),
                            );

                            if let OperationOutput::SwapTransfer(transfer_output) = &intent.outputs
                            {
                                user_account = transfer_output.dst_chain_user.clone();
                                token_out = transfer_output.token_out.clone();
                            }
                            if let OperationInput::SwapTransfer(transfer_input) = &intent.inputs {
                                token_in = transfer_input.token_in.clone();
                                amount_in = transfer_input.amount_in.clone();
                            }

                            let balance_ant = client
                                .get_token_account_balance(&usdt_token_account)
                                .await
                                .unwrap()
                                .ui_amount
                                .unwrap();

                            // swap USDT -> token_out
                            let mut ok = true;
                            if token_out != usdt_contract_address {
                                if let Err(e) = solana_executing(intent.clone(), amount).await {
                                    println!(
                                        "Error occurred on solana swap USDT -> token_out (you need to make the swap & send_funds_to_user() manually): {}",
                                        e
                                    );
                                    ok = false;
                                }
                            }

                            // send token_out -> user & user sends token_in -> solver
                            if ok {
                                if let Err(e) = solana_send_funds_to_user(
                                    intent_id,
                                    &token_in,
                                    &token_out,
                                    &user_account,
                                    intent.src_chain == intent.dst_chain,
                                )
                                .await
                                {
                                    println!(
                                            "Error occurred on send token_out -> user & user sends token_in -> solver (you need to send_funds_to_user() manually): {}",
                                            e
                                        );
                                } else if token_in != usdt_contract_address {
                                    // swap token_in -> USDT
                                    let memo = format!(
                                        r#"{{"user_account": "{}","token_in": "{}","token_out": "{}","amount": {},"slippage_bps": {}}}"#,
                                        SOLVER_ADDRESSES.get(1).unwrap(), token_in, usdt_contract_address, amount_in, 100
                                    );

                                    if let Err(e) = jupiter_swap(
                                        &memo,
                                        &client,
                                        &from_keypair,
                                        SwapMode::ExactIn,
                                    )
                                    .await
                                    {
                                        println!("Error on solana swap token_in -> USDT (you need to make the swap manually): {e}");
                                    } else {
                                        let balance_post = client
                                            .get_token_account_balance(&usdt_token_account)
                                            .await
                                            .unwrap()
                                            .ui_amount
                                            .unwrap();

                                        let balance;
                                        if balance_post >= balance_ant {
                                            balance = balance_post - balance_ant;
                                            println!(
                                                "You have win {} USDT on intent {intent_id}",
                                                balance
                                            );
                                        } else {
                                            balance = balance_ant - balance_post;
                                            println!(
                                                "You have lost {} USDT on intent {intent_id}",
                                                balance
                                            );
                                        }
                                    }
                                }
                            }
                        } else if intent.dst_chain == "ethereum" {
                            let usdt_contract_address =
                                "0xdac17f958d2ee523a2206206994597c13d831ec7";

                            let rpc_url =
                                env::var("ETHEREUM_RPC").expect("ETHEREUM_RPC must be set");
                            let private_key =
                                env::var("ETHEREUM_PKEY").expect("ETHEREUM_PKEY must be set");
                            let target_address: Address =
                                Address::from_str(SOLVER_ADDRESSES.get(0).unwrap()).unwrap();

                            let provider = match Provider::<Http>::try_from(&rpc_url) {
                                Ok(provider) => Arc::new(provider),
                                Err(e) => {
                                    println!("Failed to create Ethereum provider: {}", e);
                                    return;
                                }
                            };

                            let usdt_contract = UsdtContract::new(
                                Address::from_str(usdt_contract_address).unwrap(),
                                provider.clone(),
                            );

                            let balance_ant =
                                match usdt_contract.balance_of(target_address).call().await {
                                    Ok(balance) => balance,
                                    Err(e) => {
                                        println!("Failed to get USDT balance: {}", e);
                                        return;
                                    }
                                };

                            let mut token_in = String::default();
                            let mut token_out = String::default();

                            if let OperationOutput::SwapTransfer(transfer_output) = &intent.outputs
                            {
                                token_out = transfer_output.token_out.clone();
                            }
                            if let OperationInput::SwapTransfer(transfer_input) = &intent.inputs {
                                token_in = transfer_input.token_in.clone();
                            }

                            // swap USDT -> token_out
                            let mut ok = true;
                            if token_out != usdt_contract_address {
                                if let Err(e) =
                                    ethereum_executing(intent_id, intent.clone(), amount).await
                                {
                                    println!(
                                        "Error occurred on Ethereum swap USDT -> token_out: {}",
                                        e
                                    );
                                    ok = false;
                                }
                            }

                            // send token_out -> user & user sends token_in -> solver
                            if ok {
                                if let Err(e) = approve_erc20(&rpc_url, &private_key, &token_out, ESCROW_SC_ETHEREUM, amount).await
                                {
                                    println!("Error approving {token_out} for solver: {e}");
                                    return;
                                }

                                if let Err(e) = ethereum_send_funds_to_user(
                                    &rpc_url,
                                    &private_key,
                                    ESCROW_SC_ETHEREUM,
                                    intent_id,
                                    SOLVER_ADDRESSES.get(0).unwrap(),
                                    U256::zero(),
                                )
                                .await
                                {
                                    println!(
                                    "Error occurred on Ethereum send token_out -> user & user sends token_in -> solver: {}",
                                    e
                                );
                                    return;
                                } else if token_in != usdt_contract_address {
                                    // swap token_in -> USDT
                                    if let Err(e) = approve_erc20(&rpc_url, &private_key, &token_in, PARASWAP, amount).await
                                    {
                                        println!("Error approving {token_out} for solver: {e}");
                                        return;
                                    }

                                    let (token_out, token1_decimals) =
                                        match get_token_info("USDT", "ethereum") {
                                            Some((token_out, token1_decimals)) => {
                                                (token_out.to_string(), token1_decimals)
                                            }
                                            None => {
                                                println!(
                                                    "Failed to get token info for USDT on Ethereum"
                                                );
                                                return;
                                            }
                                        };

                                    let mut token_in = String::default();
                                    let mut amount_in = String::default();

                                    if let OperationInput::SwapTransfer(transfer_input) =
                                        &intent.inputs
                                    {
                                        token_in = transfer_input.token_in.clone();
                                        amount_in = transfer_input.amount_in.clone();
                                    }

                                    let token0_decimals = get_evm_token_decimals(&ERC20::new(
                                        Address::from_str(&token_in).unwrap(),
                                        provider.clone(),
                                    ))
                                    .await;

                                    let paraswap_params = ParaswapParams {
                                        side: "SELL".to_string(),
                                        chain_id: 1,
                                        amount_in: BigInt::from_str(&amount_in).unwrap(),
                                        token_in: Address::from_str(&token_in).unwrap(),
                                        token_out: Address::from_str(&token_out).unwrap(),
                                        token0_decimals: token0_decimals as u32,
                                        token1_decimals: token1_decimals as u32,
                                        wallet_address: Address::from_str(
                                            SOLVER_ADDRESSES.get(0).unwrap(),
                                        )
                                        .unwrap(),
                                        receiver_address: Address::from_str(
                                            SOLVER_ADDRESSES.get(0).unwrap(),
                                        )
                                        .unwrap(),
                                        client_aggregator: Client::new(),
                                    };

                                    let (_res_amount, res_data, res_to) =
                                        match simulate_swap_paraswap(paraswap_params).await {
                                            Ok(result) => result,
                                            Err(e) => {
                                                println!("Error simulating Paraswap swap: {}", e);
                                                return;
                                            }
                                        };

                                    if let Err(e) =
                                        send_tx(res_to, res_data, 1, 10_000_000, 0, rpc_url).await
                                    {
                                        println!("Error sending transaction on Ethereum: {}", e);
                                        return;
                                    }

                                    let balance_post = match usdt_contract
                                        .balance_of(target_address)
                                        .call()
                                        .await
                                    {
                                        Ok(balance) => balance,
                                        Err(e) => {
                                            println!("Failed to get post-swap USDT balance: {}", e);
                                            return;
                                        }
                                    };

                                    let balance;
                                    if balance_post >= balance_ant {
                                        balance = balance_post - balance_ant;
                                        println!(
                                            "You have won {} USDT on intent {intent_id}",
                                            balance.as_u128() as f64 / 1e6
                                        );
                                    } else {
                                        balance = balance_ant - balance_post;
                                        println!(
                                            "You have lost {} USDT on intent {intent_id}",
                                            balance.as_u128() as f64 / 1e6
                                        );
                                    }
                                }
                            }
                        }

                        // ws_sender
                        //     .send(Message::text(msg))
                        //     .await
                        //     .expect("Failed to send message");
                    }

                    {
                        let mut intents = INTENTS.write().await;
                        intents.remove(&intent_id.to_string());
                        drop(intents);
                    }
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            _ => {}
        }
    }

    println!("Auctioner went down, please reconnect");
}
