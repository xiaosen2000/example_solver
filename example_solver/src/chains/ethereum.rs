pub mod ethereum_chain {
    use crate::chains::get_token_info;
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

    abigen!(
        Escrow,
        r#"[{
            "constant": false,
            "inputs": [
                {
                    "components": [
                        { "name": "intentId", "type": "string" },
                        { "name": "solverOut", "type": "string" },
                        { "name": "singleDomain", "type": "bool" }
                    ],
                    "name": "solverTransferData",
                    "type": "tuple"
                }
            ],
            "name": "sendFundsToUser",
            "outputs": [],
            "payable": true,
            "stateMutability": "payable",
            "type": "function"
        }]"#
    );

    abigen!(
        UsdtContract,
        r#"[
            function balanceOf(address owner) view returns (uint256)
        ]"#
    );
    

    pub const ESCROW_SC_ETHEREUM: &str = "0x2ed71A143D7CC3281D51d66bb56f47A555b6F840";

    pub async fn ethereum_executing(
        intent_id: &str,
        intent: PostIntentInfo,
        amount: &str,
    ) -> String {
        let client_rpc = env::var("ETHEREUM_RPC").expect("ETHEREUM_RPC must be set");
        let mut msg = String::default();
        let mut token_out = String::default();

        match intent.function_name.as_str() {
            "transfer" => {
                if let OperationOutput::SwapTransfer(transfer_output) = &intent.outputs {
                    token_out = transfer_output.token_out.clone();
                }

                msg = match transfer_erc20(
                    &client_rpc,
                    &env::var("ETHEREUM_PKEY").expect("ETHEREUM_PKEY must be set"),
                    &token_out,
                    SOLVER_ADDRESSES.get(0).unwrap(),
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
                let (token_in, token0_decimals) = get_token_info("USDT", "ethereum").unwrap();
                let mut token_out = String::default();

                if let OperationOutput::SwapTransfer(transfer_output) = &intent.outputs {
                    token_out = transfer_output.token_out.clone();
                }

                let provider =
                    Provider::<Http>::try_from(client_rpc.replace("wss", "https")).unwrap();
                let provider = Arc::new(provider);

                let token1_decimals = get_evm_token_decimals(&ERC20::new(
                    Address::from_str(&token_out).unwrap(),
                    provider.clone(),
                ))
                .await;

                let paraswap_params = ParaswapParams {
                    side: "BUY".to_string(),
                    chain_id: 1,
                    amount_in: BigInt::from_str(amount).unwrap(),
                    token_in: Address::from_str(&token_in).unwrap(),
                    token_out: Address::from_str(&token_out).unwrap(),
                    token0_decimals: token0_decimals as u32,
                    token1_decimals: token1_decimals as u32,
                    wallet_address: Address::from_str(SOLVER_ADDRESSES.get(0).unwrap()).unwrap(),
                    receiver_address: Address::from_str(SOLVER_ADDRESSES.get(0).unwrap()).unwrap(),
                    client_aggregator: Client::new(),
                };

                let (_res_amount, res_data, res_to) =
                    simulate_swap_paraswap(paraswap_params).await.unwrap();

                msg = send_tx(res_to, res_data, 1, 10_000_000, 0, client_rpc).await;

                msg = json!({
                    "code": 3,
                    "msg": {
                        "intent_id": intent_id,
                        "tx_hash": msg
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
        let amount = U256::from_dec_str(amount).unwrap();

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
        url: String,
    ) -> String {
        let prvk = secp256k1::SecretKey::from_str(
            &env::var("ETHEREUM_PKEY").unwrap_or_else(|_| "".to_string()),
        )
        .unwrap();

        // Get gas
        let response =
            reqwest::get("https://api.etherscan.io/api?module=gastracker&action=gasoracle")
                .await
                .unwrap()
                .json::<GasResponse>()
                .await
                .unwrap();

        // Parse the propose gas price as f64
        let propose_gas_price_f64: f64 = response.result.propose_gas_price.parse().unwrap();

        // Convert to wei (1 Gwei = 1e9 wei)
        let propose_gas_price_wei: u128 = (propose_gas_price_f64 * 1e9) as u128;

        let base_fee_per_gas = propose_gas_price_wei;
        let priority_fee_per_gas: u128 = 2_000_000_000; // This is already in wei
        let max_fee_per_gas = base_fee_per_gas + priority_fee_per_gas;

        // EIP-1559
        let tx_object = web3::types::TransactionParameters {
            to: Some(to),
            gas: U256::from(gas),
            value: U256::from(value),
            data: web3::types::Bytes::from(hex::decode(data[2..].to_string()).unwrap()),
            chain_id: Some(chain_id),
            transaction_type: Some(web3::types::U64::from(2)),
            access_list: None,
            max_fee_per_gas: Some(U256::from(max_fee_per_gas)),
            max_priority_fee_per_gas: Some(U256::from(priority_fee_per_gas)),
            ..Default::default()
        };

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
        token_out: &str,
    ) -> BigInt {
        let rpc_url = env::var("ETHEREUM_RPC").expect("ETHEREUM_RPC must be set");
        let provider = Provider::<Http>::try_from(rpc_url)
            .map_err(|e| e.to_string())
            .unwrap();
        let provider = Arc::new(provider);
        let token_in = Address::from_str(token_in).unwrap();
        let token_out = Address::from_str(token_out).unwrap();
        let token0_decimals = get_evm_token_decimals(&ERC20::new(token_in, provider.clone())).await;
        let token1_decimals =
            get_evm_token_decimals(&ERC20::new(token_out, provider.clone())).await;

        let paraswap_params = ParaswapParams {
            side: "SELL".to_string(),
            chain_id: 1,
            amount_in: BigInt::from_str(amount_in).unwrap(),
            token_in: token_in,
            token_out: token_out,
            token0_decimals: token0_decimals as u32,
            token1_decimals: token1_decimals as u32,
            wallet_address: Address::from_str("0x61e3D9E355E7CeF2D685aDF4d917586f9350e298")
                .unwrap(),
            receiver_address: Address::from_str("0x61e3D9E355E7CeF2D685aDF4d917586f9350e298")
                .unwrap(),
            client_aggregator: Client::new(),
        };

        let (_res_amount, _, _) = simulate_swap_paraswap(paraswap_params).await.unwrap();
        _res_amount
    }

    pub async fn ethereum_send_funds_to_user(
        provider_url: &str,
        private_key: &str,
        contract_address: &str,
        intent_id: &str,
        solver_out: &str,
        single_domain: bool,
        value_in_wei: U256,
    ) -> Result<TransactionReceipt, Box<dyn std::error::Error>> {
        let provider = Provider::<Http>::try_from(provider_url)?;
        let provider = Arc::new(provider);

        let wallet: LocalWallet = private_key.parse()?;
        let wallet = wallet.with_chain_id(1u64); // Mainnet
        let wallet = Arc::new(SignerMiddleware::new(provider.clone(), wallet));

        let contract_address = contract_address.parse::<Address>()?;
        let contract = Escrow::new(contract_address, wallet.clone());

        let solver_transfer_data = (intent_id.to_string(), solver_out.to_string(), single_domain);

        let contract = contract
            .send_funds_to_user(solver_transfer_data)
            .value(value_in_wei);
        let pending_tx = contract.send().await?;

        let tx_receipt = pending_tx
            .await?
            .expect("Failed to fetch transaction receipt");

        Ok(tx_receipt)
    }
}
