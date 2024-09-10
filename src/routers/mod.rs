pub mod jupiter;
pub mod paraswap;

// use ethers::providers::Middleware;
use ethers::prelude::*;
use serde_json::Value;
use crate::chains::*;
use crate::PostIntentInfo;
use ethereum::ethereum_chain::{ethereum_simulate_swap, fetch_eth_gas_price};
use lazy_static::lazy_static;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use solana::solana_chain::solana_simulate_swap;
use std::collections::HashMap;
use std::env;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

// Constants for gas usage and costs
const STORE_INTENT_GAS: u64 = 250_000;
const SEND_FUNDS_TO_USER_GAS: u64 = 170_000;
const ON_RECEIVE_TRANSFER_GAS: u64 = 150_000;
const ETH_TO_SOL_BRIDGE_FEE: f64 = 0.05; // in SOL

// Struct to hold fee information
#[derive(Debug, Clone)]
struct FeeInfo {
    // In USD
    store_intent: f64, // called on des chian
    send_funds_to_user: f64, // called on des chian
    on_receive_transfer: f64, // when cross chain, called on src chain
    relayer_fee: f64, // when cross chain, send message des -> src
    // rollup_fee: f64,
}

lazy_static! {
    pub static ref FLAT_FEES: Arc<RwLock<HashMap<(String, String), FeeInfo>>> = Arc::new(RwLock::new(HashMap::new()));
}

async fn fetch_eth_price() -> Result<f64, Box<dyn std::error::Error>> {
    let url = "https://api.coingecko.com/api/v3/simple/price?ids=ethereum&vs_currencies=usd";
    let response: Value = reqwest::get(url).await.unwrap().json().await.unwrap();
    let eth_price = response["ethereum"]["usd"].as_f64().unwrap().round();
    Ok(eth_price)
}

async fn fetch_sol_price() -> Result<f64, Box<dyn std::error::Error>> {
    let url = "https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd";
    let response: Value = reqwest::get(url).await.unwrap().json().await.unwrap();
    let sol_price = response["solana"]["usd"].as_f64().unwrap().round();
    Ok(sol_price)
}

pub async fn update_flat_fees() -> Result<(), Box<dyn std::error::Error>> {
    let mut fees = FLAT_FEES.write().await;

    let eth_gas_price = fetch_eth_gas_price()
        .await
        .map_err(|e| format!("Failed to fetch gas price: {}", e))?;
    let priority_fee_per_gas: u128 = 2_000_000_000; // This is already in wei
    let max_fee_per_gas = eth_gas_price + priority_fee_per_gas;

    let eth_price = fetch_eth_price()
        .await
        .map_err(|e| format!("Failed to fetch eth price: {}", e))?;
    let sol_price = fetch_sol_price()
        .await
        .map_err(|e| format!("Failed to fetch sol price: {}", e))?;
    // Ethereum single-domain fees
    let eth_store_intent = max_fee_per_gas * eth_price / 1e18;
    let eth_send_funds = max_fee_per_gas.mul(U256::from(SEND_FUNDS_TO_USER_GAS)) * eth_price / 1e18;
    let eth_on_receive = max_fee_per_gas * eth_price / 1e18;

    // Solana single-domain fees
    // let sol_store_intent = 0.008;
    // let sol_send_funds = 0.008;

    // Cross-domain fees
    // let eth_to_sol_on_receive = 0.008;
 
    fees.insert(
        ("ethereum".to_string(), "ethereum".to_string()),
        FeeInfo {
            store_intent: eth_store_intent,
            send_funds_to_user: eth_send_funds,
            on_receive_transfer: 0.0,
            relayer_fee: 0.0,
        }
    );

    fees.insert(
        ("solana".to_string(), "solana".to_string()),
        FeeInfo {
            store_intent: 0.008,
            send_funds_to_user: 0.008,
            on_receive_transfer: 0.0,
            relayer_fee: 0.0,
        }
    );

    fees.insert(
        ("ethereum".to_string(), "solana".to_string()),
        FeeInfo {
            store_intent: 0.008,
            send_funds_to_user: 0.008,
            on_receive_transfer: eth_on_receive,
            relayer_fee: 0.0,
        }
    );

    fees.insert(
        ("solana".to_string(), "ethereum".to_string()),
        FeeInfo {
            store_intent: eth_store_intent,
            send_funds_to_user: eth_send_funds,
            on_receive_transfer: 0.008,
            relayer_fee: 0.5 * sol_price,
        }
    );

    println!("Updated FLAT_FEES: {:?}", *fees);
    Ok(())
}

pub async fn get_flat_fee(src_chain: &str, dst_chain: &str) -> Result<f64, Box<dyn std::error::Error>> {
    let fees = FLAT_FEES.read().await;
    let fee_info = fees.get(&(src_chain.to_string(), dst_chain.to_string()))
        .ok_or("Fee information not found for the given chain pair")?;

    let total_fee = fee_info.store_intent + fee_info.send_funds_to_user + fee_info.on_receive_transfer + fee_info.relayer_fee;
    Ok(total_fee)
}

pub async fn start_fee_updater() {
    tokio::spawn(async {
        loop {
            if let Err(e) = update_flat_fees().await {
                eprintln!("Error updating flat fees: {:?}", e);
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(300)).await; // Update every 5 minutes
        }
    });
}

pub async fn get_simulate_swap_intent(
    intent_info: &PostIntentInfo,
    src_chain: &str,
    dst_chain: &str,
    bridge_token: &String,
) -> String {
    // Extracting values from OperationInput
    let (token_in, amount_in) = match &intent_info.inputs {
        OperationInput::SwapTransfer(input) => (input.token_in.clone(), input.amount_in.clone()),
        OperationInput::Lend(_) => todo!(),
        OperationInput::Borrow(_) => todo!(),
    };

    let (dst_chain_user, token_out, _) = match &intent_info.outputs {
        OperationOutput::SwapTransfer(output) => (
            output.dst_chain_user.clone(),
            output.token_out.clone(),
            output.amount_out.clone(),
        ),
        OperationOutput::Lend(_) => todo!(),
        OperationOutput::Borrow(_) => todo!(),
    };

    let (bridge_token_address_src, _) = get_token_info(bridge_token, src_chain).unwrap();
    let mut amount_out_src_chain = BigInt::from_str(&amount_in).unwrap();

    if !bridge_token_address_src.eq_ignore_ascii_case(&token_in) {
        // simulate token_in -> USDT
        if src_chain == "ethereum" {
            amount_out_src_chain =
                ethereum_simulate_swap(&token_in, &amount_in, bridge_token_address_src).await;
        } else if src_chain == "solana" {
            amount_out_src_chain = BigInt::from_str(
                &solana_simulate_swap(
                    &dst_chain_user,
                    &token_in,
                    &bridge_token_address_src,
                    BigInt::from_str(&amount_in).unwrap().to_u64().unwrap(),
                )
                .await,
            )
            .unwrap();
        }
    }

    let (bridge_token_address_dst, _) = get_token_info(bridge_token, dst_chain).unwrap();

    // get flat fees
    let flat_fee = get_flat_fee(src_chain, dst_chain).await
        .unwrap_or_else(|_| 0.0);

    // get comission
    let comission = env::var("COMISSION")
        .expect("COMISSION must be set")
        .parse::<u32>()
        .unwrap();

    if amount_out_src_chain < BigInt::from(flat_fee + comission) {
        return String::from("0");
    }

    // we substract the flat fees and the solver comission in USD
    let amount_in_dst_chain = amount_out_src_chain.clone()
        - (BigInt::from(flat_fee)
            + (amount_out_src_chain * BigInt::from(comission) / BigInt::from(100_000)));
    let mut final_amount_out = amount_in_dst_chain.to_string();

    if !bridge_token_address_dst.eq_ignore_ascii_case(&token_out) {
        // simulate USDT -> token_out
        if dst_chain == "ethereum" {
            final_amount_out =
                ethereum_simulate_swap(bridge_token_address_src, &final_amount_out, &token_out)
                    .await
                    .to_string();
        } else if dst_chain == "solana" {
            final_amount_out = solana_simulate_swap(
                &dst_chain_user,
                bridge_token_address_dst,
                &token_out,
                amount_in_dst_chain.to_u64().unwrap(),
            )
            .await;
        }
    }

    final_amount_out
}

// Calculation ethereum gas fees
// let url = "https://api.coingecko.com/api/v3/simple/price?ids=ethereum&vs_currencies=usd";
// let response: Value = reqwest::get(url).await.unwrap().json().await.unwrap();
// let eth_price = response["ethereum"]["usd"].as_f64().unwrap().round();
// let gas_price = provider.get_gas_price().await.unwrap().as_u128() as f64;
// let gas =  295000f64;
// let flat_fees = (eth_price * ((gas * gas_price) / 1e18)) as f64;

// let profit = (amount_out_src_chain.to_f64().unwrap() / 10f64.powi(bridge_token_dec_src as i32))
//     - (amount_in_dst_chain.to_f64().unwrap() / 10f64.powi(bridge_token_dec_dst as i32));
