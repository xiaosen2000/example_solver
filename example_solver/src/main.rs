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
use crate::routers::paraswap::paraswap_router::simulate_swap_paraswap;
use crate::routers::paraswap::paraswap_router::ParaswapParams;
use chains::create_keccak256_signature;
use ethers::providers::{Http, Provider};
use ethers::types::Address;
use ethers::types::U256;
use futures::{SinkExt, StreamExt};
use num_bigint::BigInt;
use reqwest::Client;
use serde_json::json;
use serde_json::Value;
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

    tokio::spawn(async move {
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
                        let intent_info: PostIntentInfo =
                            serde_json::from_value(intent_value).unwrap();

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
                        if let OperationOutput::SwapTransfer(transfer_output) = &intent_info.outputs
                        {
                            amount_out_min =
                                U256::from_dec_str(&transfer_output.amount_out).unwrap();
                        }

                        let final_amount = U256::from_dec_str(&final_amount).unwrap();

                        println!("User wants {amount_out_min} token_out, you can provide {final_amount} token_out");

                        if final_amount > amount_out_min {
                            let mut json_data = json!({
                                "code": 2,
                                "msg": {
                                    "intent_id": intent_id,
                                    "solver_id": SOLVER_ID.to_string(),
                                    "amount": final_amount.to_string()
                                }
                            });

                            create_keccak256_signature(
                                &mut json_data,
                                SOLVER_PRIVATE_KEY.to_string(),
                            )
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
                                _ = solana_executing(intent_id, intent, amount).await;
                            } else if intent.dst_chain == "ethereum" {
                                let rpc_url =
                                    env::var("ETHEREUM_RPC").expect("ETHEREUM_RPC must be set");
                                let private_key =
                                    env::var("ETHEREUM_PKEY").expect("ETHEREUM_PKEY must be set");
                                let usdt_contract_address: Address =
                                    "0xdac17f958d2ee523a2206206994597c13d831ec7"
                                        .parse()
                                        .unwrap();
                                let target_address: Address =
                                    Address::from_str(SOLVER_ADDRESSES.get(0).unwrap()).unwrap();
                                let provider = Provider::<Http>::try_from(&rpc_url).unwrap();
                                let provider = Arc::new(provider);
                                let usdt_contract =
                                    UsdtContract::new(usdt_contract_address, provider.clone());

                                let balance_ant: U256 = usdt_contract
                                    .balance_of(target_address)
                                    .call()
                                    .await
                                    .unwrap();

                                // swap USDT -> token_out
                                _ = ethereum_executing(intent_id, intent.clone(), amount).await;

                                // send token_out -> user & user sends token_in -> solver
                                let _ = ethereum_send_funds_to_user(
                                    &rpc_url,
                                    &private_key,
                                    ESCROW_SC_ETHEREUM,
                                    intent_id,
                                    SOLVER_ADDRESSES.get(0).unwrap(),
                                    U256::zero(),
                                )
                                .await
                                .unwrap();

                                // swap token_in -> USDT
                                let (token_out, token1_decimals) =
                                    get_token_info("USDT", "ethereum").unwrap();
                                let mut token_in = String::default();
                                let mut amount_in = String::default();

                                if let OperationInput::SwapTransfer(transfer_input) = &intent.inputs
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
                                    simulate_swap_paraswap(paraswap_params).await.unwrap();

                                _ = send_tx(res_to, res_data, 1, 10_000_000, 0, rpc_url).await;

                                let balance_post: U256 = usdt_contract
                                    .balance_of(target_address)
                                    .call()
                                    .await
                                    .unwrap();
                                let balance;
                                if balance_post >= balance_ant {
                                    balance = balance_post - balance_ant;
                                    println!(
                                        "You have win {} USDT on intent {intent_id}",
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
    });

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    }
}
