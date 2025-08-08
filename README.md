# Raydium AMM Trading Bot

## Configuration

Create a `.env` file in the project root with the following variables:

### Required Environment Variables

```env
# Wallet Configuration
PRIVATE_KEY=your_base58_private_key_here

# Pool Configuration
POOL_ADDRESS=your_raydium_pool_address_here

# Trading Parameters
BUY_SOL_AMOUNT=0.1
ENTRY_PERCENT=5.0
SLIPPAGE=1.0

# RPC Configuration
RPC_ENDPOINT=https://your-rpc-endpoint.com
GEYSER_URL=your_yellowstone_grpc_endpoint
X_TOKEN=your_grpc_token

# MEV Service Configuration
CONFIRM_SERVICE=NOZOMI  # Options: NOZOMI, JITO, ZSLOT

# Priority Fee Configuration
CU=200000
PRIORITY_FEE_MICRO_LAMPORT=1000
```

### Environment Variables Explained

| Variable | Description | Default | Example |
|----------|-------------|---------|---------|
| `PRIVATE_KEY` | Your wallet's base58 private key | Required | `4xQy...` |
| `POOL_ADDRESS` | Raydium AMM V4 pool address to monitor | Required | `Pool...` |
| `BUY_SOL_AMOUNT` | Amount of SOL to trade | Required | `0.1` |
| `ENTRY_PERCENT` | Price drop percentage to trigger trade | `100` | `5` |
| `SLIPPAGE` | Slippage tolerance percentage | `100` | `10` |
| `RPC_ENDPOINT` | Solana RPC endpoint | Required | `https://...` |
| `GEYSER_URL` | Yellowstone gRPC endpoint | Required | `https://...` |
| `X_TOKEN` | gRPC authentication token | Required | `token...` |
| `CONFIRM_SERVICE` | MEV service to use | `NOZOMI` | `JITO` |
| `CU` | Compute units for transaction | `0` | `200000` |
| `PRIORITY_FEE_MICRO_LAMPORT` | Priority fee in micro lamports | `0` | `1000` |

## Usage

1. Configure your `.env` file with the required parameters
2. Run the bot:
```bash
cargo build --release
cargo run --release
```

The bot will:
1. Initialize connections to RPC and gRPC services
2. Start monitoring the specified pool address
3. Display current configuration and wallet information
4. Monitor pool price changes every 400ms
5. Execute trades when price drops below the entry percentage
6. Submit transactions through the configured MEV service

## Trading Logic

The bot implements the following trading strategy:

1. **Price Monitoring**: Continuously monitors pool price changes
2. **Entry Condition**: Triggers when price drops by the configured `ENTRY_PERCENT`
3. **Trade Execution**: Executes a swap transaction with the specified `BUY_SOL_AMOUNT`
4. **Slippage Protection**: Uses the configured `SLIPPAGE` parameter to protect against price movement
5. **MEV Protection**: Submits transactions through MEV services for better execution

## Supported MEV Services

### Nozomi
- Fast transaction submission
- Configurable priority fees
- Built-in tip functionality

### Jito
- MEV-optimized transaction processing
- Bundle support for better execution

### ZSlot
- Alternative MEV service
- Similar functionality to other services

## Project Structure

```
src/
├── main.rs              # Main application entry point
├── lib.rs               # Module declarations
├── config/              # Configuration management
│   ├── credentials.rs   # Wallet and endpoint configuration
│   ├── trade_setting.rs # Trading parameters
│   └── clients.rs       # MEV service clients
├── instructions/        # Trading instruction builders
│   ├── swap_base_in.rs  # Base-in swap instructions
│   └── swap_base_out.rs # Base-out swap instructions
├── service/             # MEV service implementations
├── utils/               # Utility functions
│   ├── blockhash.rs     # Blockhash management
│   ├── build_and_sign.rs # Transaction building
│   └── parse.rs         # Data parsing utilities
└── error/               # Error handling
```
