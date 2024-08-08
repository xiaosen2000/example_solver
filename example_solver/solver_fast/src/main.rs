mod chains;
mod routers;

use crate::chains::ethereum::ethereum_chain::ethereum_executing;
use crate::chains::solana::solana_chain::solana_executing;
use crate::chains::OperationOutput;
use crate::chains::PostIntentInfo;
use crate::chains::INTENTS;
use crate::chains::SOLVER_ADDRESSES;
use crate::chains::SOLVER_ID;
use crate::chains::SOLVER_PRIVATE_KEY;
use crate::routers::get_simulate_swap_intent;
use chains::create_keccak256_signature;
use ethers::types::U256;
use futures::{SinkExt, StreamExt};
use serde_json::json;
use serde_json::Value;
use std::env;
use std::str::FromStr;
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
                            .and_then(Value::as_str)
                            .unwrap();
                        let intent_value: Value = serde_json::from_str(intent_str).unwrap();
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
                            amount_out_min = U256::from_str(&transfer_output.amount_out).unwrap();
                        }

                        let final_amount = U256::from_str(&final_amount).unwrap();
                        if final_amount > amount_out_min {
                            let mut json_data = json!({
                                "code": 2,
                                "msg": {
                                    "intent_id": intent_id,
                                    "solver_id": SOLVER_ID.to_string(),
                                    "amount": final_amount
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
                        let mut msg = parsed
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
                                msg = solana_executing(intent_id, intent).await;
                            } else if intent.dst_chain == "ethereum" {
                                msg = ethereum_executing(intent_id, intent).await;
                            }

                            ws_sender
                                .send(Message::text(msg))
                                .await
                                .expect("Failed to send message");
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
