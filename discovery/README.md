# Discovery Service

## TODO:
- [ ] Ability to get nodes for validation
- [ ] Rate limiting
- [ ] Linting
- [ ] Pre-commit hooks & github actions 
- [ ] Cache compute pools from chain 

## Sending Requests with Signature Creation
To send requests that require signature verification, follow these steps:

1. **Create a Message**: Construct a message that includes the request path and the payload. For example, if your request is to get a node's details:
   - URL Path: `/nodes/0x1234567890abcdef`
   - Payload: `{ "someKey": "someValue" }`
   The message would be:
   """
   const message = '/nodes/0x1234567890abcdef' + JSON.stringify({ "someKey": "someValue" });
   """

2. **Sign the Message**: Use the user's private key to sign the message:
   """
   const signature = await wallet.signMessage(message);
   """

3. **Include in Request Headers**: Add the signature and the user's Ethereum address in the request headers:
   - `x-eth-address`: The user's Ethereum address (e.g., `0x1234567890abcdef1234567890abcdef12345678`).
   - `x-signature`: The signed message.

Make sure to include these headers in your requests to ensure proper authentication and authorization.

## Blocked for testnet:
- [ ] Release & deployment 

## Development
Run tests:
```
docker compose run test
```