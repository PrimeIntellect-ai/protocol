# Discovery Service

## TODO:
- [ ] Better logging
- [ ] Protocol Service Access via custom API Key
- [ ] Ability to test compute pool request 
- [ ] Pre-commit hooks & github actions 

- [ ] Rate limiting: PRI-906 
- [ ] Better testing


## Blocked until testnet:
- [ ] Deployment
- [ ] Cache compute pools from chain 

## Sending Requests with Signature Creation
To send requests that require signature verification, follow these steps:

1. **Create a Message**: Construct a message that includes the request path and the payload. For example, if your request is to get a node's details:
   - URL Path: `/nodes/0x1234567890abcdef`
   - Payload: `{ "someKey": "someValue" }`
   The message would be:
  ```
   const message = '/nodes/0x1234567890abcdef' + JSON.stringify({ "someKey": "someValue" });
  ``` 

2. **Sign the Message**: Use the user's private key to sign the message:
  ``` 
   const signature = await wallet.signMessage(message);
  ``` 

3. **Include in Request Headers**: Add the signature and the user's Ethereum address in the request headers:
   - `x-address`: The user's Ethereum address (e.g., `0x1234567890abcdef1234567890abcdef12345678`).
   - `x-signature`: The signed message.

Make sure to include these headers in your requests to ensure proper authentication and authorization.

## Development
Run tests:
```
docker compose run test
```