pub mod ethereum_chain {
    use crate::chains::OperationInput;
    use crate::chains::OperationOutput;
    use crate::env;
    use crate::json;
    use crate::routers::paraswap::paraswap_router::simulate_swap_paraswap;
    use crate::routers::paraswap::paraswap_router::ParaswapParams;
    use crate::PostIntentInfo;
    use crate::SOLVER_ADDRESSES;
    use crate::SOLVER_ID;
    use ethers::prelude::abigen;
    use ethers::prelude::*;
    use ethers::providers::{Http, Provider};
    use num_bigint::BigInt;
    use reqwest::Client;
    use serde::Deserialize;
    use std::str::FromStr;
    use std::sync::Arc;

    #[derive(Deserialize)]
    struct GasPrice {
        #[serde(rename = "SafeGasPrice")]
        _safe_gas_price: String,
        #[serde(rename = "ProposeGasPrice")]
        propose_gas_price: String,
        #[serde(rename = "FastGasPrice")]
        _fast_gas_price: String,
    }

    #[derive(Deserialize)]
    struct GasResponse {
        result: GasPrice,
    }

    abigen!(
        ERC20,
        r#"[{
            "constant": true,
            "inputs": [],
            "name": "decimals",
            "outputs": [{ "name": "", "type": "uint8" }],
            "type": "function"
        },
        {
            "constant": false,
            "inputs": [
                { "name": "_to", "type": "address" },
                { "name": "_value", "type": "uint256" }
            ],
            "name": "transfer",
            "outputs": [{ "name": "", "type": "bool" }],
            "type": "function"
        }]"#
    );

    pub async fn ethereum_executing(intent_id: &str, intent: PostIntentInfo) -> String {
        let client_rpc = env::var("ETHEREUM_RPC").expect("ETHEREUM_RPC must be set");
        let mut msg = String::default();
        let mut user_account = String::default();
        let mut token_out = String::default();
        let mut amount = 0u64;

        match intent.function_name.as_str() {
            "transfer" => {
                if let OperationOutput::SwapTransfer(transfer_output) = &intent.outputs {
                    user_account = transfer_output.dst_chain_user.clone();
                    amount = transfer_output.amount_out.parse::<u64>().unwrap();
                    token_out = transfer_output.token_out.clone();
                }

                msg = match transfer_erc20(
                    &client_rpc,
                    &env::var("ETHEREUM_PKEY").expect("ETHEREUM_PKEY must be set"),
                    &token_out,
                    &user_account,
                    &amount.to_string(),
                )
                .await
                {
                    Ok(signature) => json!({
                        "code": 1,
                        "msg": {
                            "intent_id": intent_id,
                            "solver_id": SOLVER_ID.to_string(),
                            "tx_hash": signature,
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

                let provider =
                    Provider::<Http>::try_from(client_rpc.replace("wss", "https")).unwrap();
                let provider = Arc::new(provider);

                let token0_decimals = get_evm_token_decimals(&ERC20::new(
                    Address::from_str(&token_in).unwrap(),
                    provider.clone(),
                ))
                .await;
                let token1_decimals = get_evm_token_decimals(&ERC20::new(
                    Address::from_str(&token_out).unwrap(),
                    provider.clone(),
                ))
                .await;

                let paraswap_params = ParaswapParams {
                    side: "SELL".to_string(),
                    chain_id: 1,
                    amount_in: BigInt::from(amount),
                    token_in: Address::from_str(&token_in).unwrap(),
                    token_out: Address::from_str(&token_out).unwrap(),
                    token0_decimals: token0_decimals as u32,
                    token1_decimals: token1_decimals as u32,
                    wallet_address: Address::from_str(&SOLVER_ADDRESSES[0]).unwrap(),
                    receiver_address: Address::from_str(&user_account).unwrap(),
                    client_aggregator: Client::new(),
                };

                let (_res_amount, res_data, res_to) =
                    simulate_swap_paraswap(paraswap_params).await.unwrap();

                msg = send_tx(res_to, res_data, 1, 10_000_000, 0, client_rpc).await;
            }
            _ => {
                println!("function not supported")
            }
        }

        msg
    }

    async fn transfer_erc20(
        provider_url: &str,
        private_key: &str,
        token_address: &str,
        recipient_address: &str,
        amount: &str,
    ) -> Result<TxHash, Box<dyn std::error::Error>> {
        let provider = Provider::<Http>::try_from(provider_url)?;
        let provider = Arc::new(provider);

        let wallet: LocalWallet = private_key.parse()?;
        let wallet = wallet.with_chain_id(1u64); // Mainnet
        let wallet = Arc::new(SignerMiddleware::new(provider.clone(), wallet));

        let token_address = token_address.parse::<Address>()?;
        let erc20 = ERC20::new(token_address, wallet.clone());

        let recipient: Address = recipient_address.parse()?;
        let amount = U256::from_str(amount).unwrap();

        let tx = erc20.transfer(recipient, amount);
        let tx = tx.send().await?;

        Ok(tx.tx_hash())
    }

    pub async fn send_tx(
        to: Address,
        data: String,
        chain_id: u64,
        gas: u64,
        value: u128,
        mut url: String,
    ) -> String {
        let prvk = secp256k1::SecretKey::from_str(
            &env::var("ETHEREUM_PKEY").unwrap_or_else(|_| "".to_string()),
        )
        .unwrap();

        // Get gas shit
        let response =
            reqwest::get("https://api.etherscan.io/api?module=gastracker&action=gasoracle")
                .await
                .unwrap()
                .json::<GasResponse>()
                .await
                .unwrap();
        // let safe_gas_price: u64 = response.result.safe_gas_price.parse().unwrap();
        let propose_gas_price: u64 = response.result.propose_gas_price.parse().unwrap();
        // let fast_gas_price: u64 = response.result.fast_gas_price.parse().unwrap();
        let propose_gas_price_wei = propose_gas_price * 1_000_000_000;
        let base_fee_per_gas = propose_gas_price_wei;
        let priority_fee_per_gas = 2_000_000_000;
        let max_fee_per_gas = base_fee_per_gas + priority_fee_per_gas;

        // EIP-1559
        let tx_object = web3::types::TransactionParameters {
            to: Some(to),
            gas: web3::types::U256::from(gas),
            value: web3::types::U256::from(value),
            data: web3::types::Bytes::from(hex::decode(data[2..].to_string()).unwrap()),
            chain_id: Some(chain_id),
            transaction_type: Some(web3::types::U64::from(2)),
            access_list: None,
            max_fee_per_gas: Some(web3::types::U256::from(max_fee_per_gas)),
            max_priority_fee_per_gas: Some(web3::types::U256::from(priority_fee_per_gas)),
            ..Default::default()
        };

        url = url.replace("wss://", "https://");
        let web3_query = web3::Web3::new(web3::transports::Http::new(&url).unwrap());
        let signed = web3_query
            .accounts()
            .sign_transaction(tx_object, &prvk)
            .await
            .unwrap();

        let _tx_hash = web3_query
            .eth()
            .send_raw_transaction(signed.raw_transaction)
            .await
            .unwrap();

        _tx_hash.to_string()
    }

    pub async fn get_evm_token_decimals(erc20: &ERC20<Provider<Http>>) -> u8 {
        match erc20.decimals().call().await {
            Ok(decimals) => decimals,
            Err(e) => {
                eprintln!("Error getting decimals: {}", e);
                0
            }
        }
    }

    pub async fn ethereum_simulate_swap(
        token_in: &str,
        amount_in: &str,
        bridge_token_address_src: &str,
        bridge_token_dec_src: u32,
    ) -> BigInt {
        let provider = Provider::<Http>::try_from(
            env::var("ETHEREUM_RPC")
                .expect("ETHEREUM_RPC must be set")
                .replace("wss", "https"),
        )
        .unwrap();
        let provider = Arc::new(provider);
        let token_in = Address::from_str(&format!("0x{}", token_in)).unwrap();
        let token0_decimals = get_evm_token_decimals(&ERC20::new(token_in, provider.clone())).await;

        let paraswap_params = ParaswapParams {
            side: "SELL".to_string(),
            chain_id: 1,
            amount_in: BigInt::from_str(amount_in).unwrap(),
            token_in: token_in,
            token_out: Address::from_str(&format!("0x{}", bridge_token_address_src)).unwrap(),
            token0_decimals: token0_decimals as u32,
            token1_decimals: bridge_token_dec_src,
            wallet_address: Address::from_str("0x61e3D9E355E7CeF2D685aDF4d917586f9350e298")
                .unwrap(),
            receiver_address: Address::from_str("0x61e3D9E355E7CeF2D685aDF4d917586f9350e298")
                .unwrap(),
            client_aggregator: Client::new(),
        };

        let (_res_amount, _, _) = simulate_swap_paraswap(paraswap_params).await.unwrap();
        _res_amount
    }
}
