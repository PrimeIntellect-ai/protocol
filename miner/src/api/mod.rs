// Folder structure for the API module:
//
// api/
// ├── mod.rs (this file)
// ├── routes/
// │   ├── mod.rs
// │   └── task.rs
// ├── handlers/
// │   ├── mod.rs
// │   └── task.rs
// ├── models/
// │   ├── mod.rs
// │   └── task.rs
// ├── middleware/
// │   ├── mod.rs
// │   ├── auth.rs
// │   └── logging.rs
// └── server.rs
//
// Key responsibilities:
// - routes/: Route definitions and URL mapping
// - handlers/: Business logic for handling requests
// - models/: Data structures and types
// - middleware/: Request middleware (auth, logging, etc)
// - server.rs: HTTP server setup and configuration
//
// The HTTP server is started from server.rs which owns all API functionality
// and configuration like TLS, rate limiting, etc.

mod routes;
mod server;
