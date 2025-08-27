# Raydium AMM Monitor Backend API

This backend provides HTTP endpoints to access metadata and status information from the Raydium AMM trading bot.

## Configuration

Set the `BACKEND_PORT` environment variable to specify the port (default: 3000):

```bash
export BACKEND_PORT=3000
```

## API Endpoints

### Health Check
**GET** `/health`

Returns the health status of the backend server.

**Response:**
```json
{
  "status": "healthy",
  "timestamp": 1703123456789
}
```

### Metadata
**GET** `/metadata`

Returns comprehensive metadata about the current state of the trading bot.

**Response:**
```json
{
  "pool_price": 0.00123456,
  "latest_pool_price": 0.00123456,
  "is_bought": false,
  "bought_price": null,
  "bought_at_time": null,
  "swap_instructions_count": 0,
  "timestamp": 1703123456789
}
```

### Pool Price
**GET** `/pool-price`

Returns current and previous pool prices with percentage change.

**Response:**
```json
{
  "current_price": 0.00123456,
  "previous_price": 0.00120000,
  "price_change_percent": 2.88,
  "timestamp": 1703123456789
}
```

### Status
**GET** `/status`

Returns detailed trading status information.

**When monitoring (not bought):**
```json
{
  "status": "monitoring",
  "current_price": 0.00123456,
  "timestamp": 1703123456789
}
```

**When bought:**
```json
{
  "status": "bought",
  "bought_price": 0.00120000,
  "current_price": 0.00123456,
  "percent_change": 2.88,
  "time_elapsed_seconds": 300,
  "timestamp": 1703123456789
}
```

### Transactions
**GET** `/transactions`

Returns the last 100 transaction metadata records processed by the bot.

### Wallet Information
**GET** `/wallet`

Returns detailed information about the wallet including SOL balance, WSOL balance, and all token balances.

**Response:**
```json
{
  "pubkey": "587GsL34W8nGsZWAjTiT6dXLJSrwiM6grWyxQHSgW1eM",
  "sol_balance": 0.123456789,
  "wsol_balance": {
    "mint": "So11111111111111111111111111111111111111112",
    "symbol": null,
    "balance": "1000000000",
    "ui_balance": 1.0,
    "decimals": 9
  },
  "token_balances": [
    {
      "mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
      "symbol": null,
      "balance": "1000000",
      "ui_balance": 1.0,
      "decimals": 6
    }
  ],
  "total_tokens": 1,
  "timestamp": 1703123456789
}
```

**Response:**
```json
{
  "transactions": [
    {
      "transaction_metadata": {
        "signature": "5J7X...",
        "slot": 123456789,
        "err": null,
        "memo": null,
        "block_time": 1703123456,
        "meta": {
          "err": null,
          "fee": 5000,
          "pre_balances": [...],
          "post_balances": [...],
          "inner_instructions": [...],
          "log_messages": [...],
          "pre_token_balances": [...],
          "post_token_balances": [...],
          "rewards": [...],
          "loaded_addresses": {...},
          "return_data": null,
          "compute_units_consumed": 200000
        },
        "message": {
          "header": {...},
          "account_keys": [...],
          "recent_blockhash": "...",
          "instructions": [...],
          "address_table_lookups": [...]
        }
      }
    }
  ],
  "count": 1,
  "timestamp": 1703123456789
}
```

## Usage Examples

### Using curl
```bash
# Health check
curl http://localhost:3000/health

# Get current metadata
curl http://localhost:3000/metadata

# Get pool price information
curl http://localhost:3000/pool-price

# Get trading status
curl http://localhost:3000/status

# Get recent transactions
curl http://localhost:3000/transactions

# Get wallet information
curl http://localhost:3000/wallet
```

### Using JavaScript/Fetch
```javascript
// Get current status
const response = await fetch('http://localhost:3000/status');
const status = await response.json();
console.log('Trading status:', status);

// Get pool price
const priceResponse = await fetch('http://localhost:3000/pool-price');
const priceData = await priceResponse.json();
console.log('Current price:', priceData.current_price);
console.log('Price change:', priceData.price_change_percent + '%');

// Get wallet information
const walletResponse = await fetch('http://localhost:3000/wallet');
const walletData = await walletResponse.json();
console.log('Wallet pubkey:', walletData.pubkey);
console.log('SOL balance:', walletData.sol_balance);
console.log('WSOL balance:', walletData.wsol_balance?.ui_balance || 0);
console.log('Total tokens:', walletData.total_tokens);
```

## CORS Support

The API includes CORS headers to allow cross-origin requests from web applications. All origins are allowed by default.

## Error Handling

All endpoints return appropriate HTTP status codes:
- `200 OK` - Successful response
- `500 Internal Server Error` - Server error

## Rate Limiting

Currently, there is no rate limiting implemented. Consider implementing rate limiting for production use.

## Security Notes

- The API is designed for internal use and monitoring
- Consider adding authentication for production deployments
- The server binds to `0.0.0.0` by default - ensure proper firewall rules
