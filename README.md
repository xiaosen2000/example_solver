# 🎉 MANTIS V0: Decentralized Cross-Chain Intents 🎉

[![License](https://img.shields.io/npm/l/svelte.svg)](LICENSE.md)
[![Twitter Follow](https://img.shields.io/twitter/follow/mantis?style=social)](https://x.com/mantis)
[![Website](https://img.shields.io/badge/website-ComposableFoundation-blue)](https://www.composablefoundation.com/)

## What is Auctioneer?

The **Auctioneer** is a third-party off-chain entity that plays a crucial role in facilitating interactions between users, solvers, and the rollup within a decentralized and trustless system. Here’s an overview of how the Auctioneer operates:

## 🔄 Interaction Flow

1. **👥 User Submits Intents**
   - Users submit their intents to the Auctioneer. An intent specifies the details of a transaction, including the source chain, destination chain, and other relevant parameters. This submission is the initial step in the process.

2. **📣 Auctioneer Broadcasts Intents to Solvers**
   - Once the Auctioneer receives an intent from a user, it broadcasts this intent to solvers who are listening via WebSocket. Solvers are entities that have the capability to execute the transactions described in the intents.

3. **🤔 Solvers Decide to Participate**
   - Solvers receive the intents and decide whether to participate in the auction for executing the transaction. Each solver evaluates the intent and determines if it can provide a competitive quote and successfully execute the transaction.

4. **🏆 Auctioneer Determines Winning Solver**
   - After receiving bids from participating solvers, the Auctioneer determines which solver has won the auction. The criteria for winning can include the best quote, speed, and reliability. The Auctioneer then interacts with the escrow contract on the destination chain to announce the winning solver.

5. **⚙️ Solver Executes Transaction on Destination Chain**
   - The winning solver submits a transaction on the destination chain through the escrow contract to transfer funds to the user. If the source chain and destination chain are different, the solver also sends a cross-chain message as part of the same transaction. This ensures that the transaction is recognized and processed correctly across both chains.

6. **📦 Transaction Storage in Rollup**
   - Once the transaction is executed, whether it is a cross-chain transaction or a single-domain transaction, it is stored in the rollup. The rollup is a layer that aggregates multiple transactions and stores them securely. It also maintains the logic of how the Auctioneer operates, ensuring that the entire process remains decentralized and trustless.

7. **🔐 Decentralization and Trustlessness**
   - The rollup is responsible for storing information and executing the logic that governs the Auctioneer's operations. This setup ensures that the system remains decentralized and trustless, meaning that no single entity has control over the process, and all actions can be verified independently by participants in the network.

By managing the flow of intents, broadcasting them to solvers, determining winners, and ensuring transactions are executed and stored properly, the Auctioneer facilitates seamless and secure interactions within a decentralized ecosystem.

# Solver Setup Instructions

## Step 1: Fill the .env File
The first thing you need to do is fill out the `.env` file. Use the provided `env.example` as a template:
```bash
ETHEREUM_RPC="" # ws
ETHEREUM_PKEY=""
SOLANA_RPC="" # https
SOLANA_KEYPAIR=""
BRIDGE_TOKEN="USDT" # USDT or PICA
COMISSION="10" # if COMISSION == "1"-> 0.01%
SOLVER_ID="" # Given by Composable
COMPOSABLE_ENDPOINT="" # ws IP address Given by Composable
SOLVER_PRIVATE_KEY="" # ETH private_key
```
## Step 2: Provide Gas on Chains
The solver must provide some gas on the chains they want to operate. This gas is needed for the auctioner to perform operations such as declaring the auction winner and updating the highest bid, all on-chain. Note that gas is only required on the destination chain where the user intends to receive the token_out of their intent.
- On Ethereum, $100 is sufficient to solve about 10-15 intents.
- On Solana, with 1$ is enough to make more than 100 even 1000 intetns

## Step 3: Check Remaining Gas in the Auctioner
You can check how much gas is remaining in the auctioner by making this HTTP request:
```bash
// TO DO
```

## Step 4: Run the Solver
To run the solver, use the following command:
```sh
cargo run --release
```

Inside the `example_solver`, we have two main folders: `routers` and `chains`.

### Routers
In the `routers` folder, we have Jupiter on Solana and Paraswap on Ethereum mainnet. Feel free to add more routers or your own router system. The `routers` folder doesn't need modifications unless you want to add new routers or your own router.

### Chains
In the `chains` folder, we have two chains: Ethereum and Solana. The structure is the same for each chain. The important functions are:
- `chain_simulate_swap()`
- `chain_executing()`

#### `chain_simulate_swap()`
This function is used to participate in the auction. Inside this function, you will find the logic to simulate swaps on Jupiter for Solana and Paraswap for Ethereum. Feel free to change this if you want to add more routers.

#### `chain_executing()`
This function is used when the solver wins the auction and is solving the intent. Inside this function, you will find the process to make a swap on Paraswap or Jupiter. Feel free to change this as well.



## 🌐 Auctioner Interaction with User (HTTP)

**Composable Endpoint:**  
`http://16.171.172.151:80` // 🏗️  upgrading to V1

### Submit an Intent  
`/submit_intent`

```bash
curl -X POST http://composable_endpoint/submit_intent \
     -H "Content-Type: application/json" \
     -d '{
           "function_name": "transfer",
           "src_chain": "ethereum",
           "dst_chain": "solana",
           "inputs": {
               "SwapTransfer": {
                   "token_in": "0xdAC17F958D2ee523a2206206994597C13D831ec7",
                   "amount_in": "100",
                   "src_chain_user": "0xfD8877F8AEE747a39298E6fDE2249D01d1EEfAC8",
                   "timeout": "10000000000000000000000"
               }
           },
           "outputs": {
               "SwapTransfer": {
                   "token_out": "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
                   "amount_out": "80",
                   "dst_chain_user": "BrCjdUjqSL25DHKbHaE4wq2PEDm3UzVF7eXL6VAzVu7m"
               }
           }
         }'
```

### Response:
**OK:**
```bash
{
   "code": 1,
   "msg": {
             "intent_id": "A1B2C3D4E"
          }
}
```

**ERROR:**
```bash
{
   "code": 0,
   "msg": "Error parsing PostIntentSwapInfo"
}
```

### Prove Funds were Sent from User to Escrow SC:
`/prove_intent`

```bash
curl -X POST http://composable_endpoint/prove_intent \
     -H "Content-Type: application/json" \
     -d '{
           "intent_id": "A1B2C3D4E",
           "tx_hash": "0x0839f0543be271a9f62f038b28ecb0ea151c2ddf5bda90533bb8eb4c46bf8be8"
         }'
```

### Response:
**OK:**
```bash
{
   "code": 2,
   "msg": "Intent successfully proved"
}
```

**ERROR:**
```bash
{
   "code": 0,
   "msg": error_msg // "Error, Intent not proved" OR "Error parsing IntentProof"
}
```

## 🌐 Auctioner Interaction with Solver (WS)

**Composable Endpoint:**  
`ws://16.171.172.151:443` 🏗️  upgrading to V1

### Register Solver Addresses (one address per chain)
```bash
{        
   "code": 1,
   "msg": {
             "solver_id": SOLVER_ID, // Given by Composable
             "solver_addresses": SOLVER_ADDRESSES, // vec!(solana address, ethereum address, ...)
             "intent_hash": "...", // Keccak256Hash of the intent 
             "signature": "..." // ECDSA signature of the hash
          }
}
```
### Response:
**OK:**
```bash
{
   "code": 3,
   "msg": "Solver was successfully registered"
}
```

**ERROR:**
```bash
{
   "code": 0,
   "msg": "Error checking the solver signature"
}
```

### Participate in an Intent Auction:
```bash
{
   "code": 2,
   "msg": {
             "intent_id": intent_id, // obtained listening to Intents
             "solver_id": SOLVER_ID, // Given by Composable
             "amount": "...", // off-chain solver setup to get the best quote
             "intent_hash": "...", // Keccak256Hash of the intent 
             "signature": "..." // ECDSA signature of the hash      
          }
}
```

### Response:
**OK:**
```bash
{
   "code": 4,
   "msg": msg // "You won this auction!"
              // OR "You lost this auction"
}
```

**ERROR:**
```bash
{
   "code": 0,
   "msg": error_msg // "Auction finished for this intent_id"
                    // OR "No auction found for this intent_id"
                    // OR "No solver participated in this auction"
}
```





